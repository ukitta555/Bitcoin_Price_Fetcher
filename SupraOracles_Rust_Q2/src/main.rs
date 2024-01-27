use std::{env, fs::read_to_string, io::{self, stdout, Read, Write}, process::{Child, Command, Stdio}, thread, time::{Duration, SystemTime, UNIX_EPOCH}};
use serde::{Deserialize, Serialize};
use clap::Parser;
use url::Url;
use tungstenite::{connect, stream::MaybeTlsStream, Error, Message, WebSocket};

#[derive(Parser, Debug)]
struct Args {
  #[arg(long)]
  mode: String,
  #[arg(long, value_parser = clap::value_parser!(u16).range(1..))] // protect against division by 0
  times: Option<u16>,
  #[arg(long)]
  kickoff: Option<u64>
}

#[derive(Serialize, Deserialize)]
struct Response {
  result: ResultNested
}

#[derive(Serialize, Deserialize)]
struct ResultNested {
  price: String
}

fn query_binance_api<S>(socket: &mut WebSocket<MaybeTlsStream<S>>) -> Result<String, Error>
where S: Read + Write 
{
  let _ = socket.write_message(Message::Text(r#"{
    "id": "043a7cf2-bde3-4888-9604-c8ac41fcba4d",
    "method": "ticker.price",
    "params": {
      "symbol": "BTCUSDT"
    }
  }      
  "#.into()))?;


  let msg = socket.read_message().expect("Error reading message");
  Result::Ok(msg.to_string())
}

fn main() {
  let args = Args::parse();
  let mode = args.mode;
  
  match &mode[..] {
    "cache" => {
      let mut stdout_handler = stdout();
      let kickoff = args.kickoff.expect("Kickoff time was not provided!");

      // couldn't think of something better than just busy waiting 
      let mut current_time = SystemTime::now().duration_since(UNIX_EPOCH)
        .expect("Could not fetch current time!").as_secs();
      while current_time < kickoff {
        current_time = SystemTime::now().duration_since(UNIX_EPOCH)
        .expect("Could not fetch current time!").as_secs();
      }

      let seconds = args.times.expect("In --cache mode, you should provide the number of times you want to query the Binance WS API.");
      let (mut socket, _response) = connect(
          Url::parse("wss://ws-api.binance.com:443/ws-api/v3").unwrap()
      ).expect("Can't connect");

      
      let mut average: f64 = 0.0;
      let mut data_points: Vec<f64> = Vec::new();
      for _ in 0..seconds {
        let api_response = query_binance_api(&mut socket).expect("Error while querying Binance API");
        let parsed_json: Response = serde_json::from_str(&api_response[..])
          .expect("Binance WS API returned an incorrectly formatted JSON string");

        let current_price = parsed_json.result.price.parse::<f64>().expect("Binance WS API returned a price that is not a float");
        average += current_price;
        data_points.push(current_price); // don't perform writing to file while fetching data
        thread::sleep(std::time::Duration::new(1, 0));
      }
      average /= f64::from(seconds);
      write!(stdout_handler, "{:.20}", average).expect("Failed to write to stdout.");
    }
    "read" => {
      for line in read_to_string("cache.txt").unwrap().lines() {
        println!("{}", line.to_string());
      }
      let _ = io::stdout().flush();
    }
    "aggregator" => {   
      let mut children: Vec<Child> = Vec::new();
      let number_of_children = 5;
      
      let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards");
      let three_seconds_later = now.checked_add(Duration::from_secs(3)).expect("Overflow when adding 3 seconds!");

      for _ in 0..number_of_children {
        children.push(
          Command::new("sh")
          .stdin(Stdio::piped())
          .stdout(Stdio::piped())
          .arg("-c")
          .arg(
            format!(
              "cd {} && ./target/debug/SupraOracles_Rust_2 --mode=cache --times=10 --kickoff={}", 
              env::current_dir().unwrap().to_str().unwrap().to_string(),
              three_seconds_later.as_secs()
            ),
          )
          .spawn()
          .expect("failed to spawn child process")
        );
      }

      let mut average_of_averages = 0.0;
      for child in children.iter_mut() {
        let exec_result = child.wait().unwrap();
        if !exec_result.success() {
            panic!("One of the child processes terminated with a non-zero status code! Please search for bugs.");
        }        
        let mut s: String = String::new();
        match child.stdout.as_mut().unwrap().read_to_string(&mut s) {
            Err(why) => panic!("couldn't read stdout: {}", why),
            Ok(_) => {
              // println!("Child responded with: {}", s);
              average_of_averages += s.parse::<f64>().expect("Child process sent a string that is not a float!");
            },
        }
      }

      average_of_averages /= f64::from(number_of_children);
      println!("Price of BTC in USD (average of {} averages): {:.20}", number_of_children, average_of_averages);
    } 
    _ => {
      panic!("Wrong mode value passed!")
    }
  } 
}
