// Run command: cargo run -- --mode=aggregator

use std::{env, fs::read_to_string, io::{self, stdout, Read, Write}, process::{Child, Command, Stdio}, thread, time::{Duration, SystemTime, UNIX_EPOCH}};
use k256::{ecdsa::{SigningKey, Signature, VerifyingKey, signature::Signer}, schnorr::signature::Verifier, EncodedPoint};
use rand_core::OsRng;
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

        // match parsed_json {
        //   Ok(res) => {...} 
        //   Err(_) => { print!();}
        // }
        
        let current_price = parsed_json.result.price.parse::<f64>().expect("Binance WS API returned a price that is not a float");
        average += current_price;
        data_points.push(current_price); // don't perform writing to file while fetching data
        thread::sleep(std::time::Duration::new(1, 0));
      }
      average /= f64::from(seconds);

      let signing_key = SigningKey::random(&mut OsRng);
      let _sk = signing_key.to_bytes();
      let verify_key = VerifyingKey::from(&signing_key);
      let vk = verify_key.to_encoded_point(false);
      stdout_handler.write_all(vk.as_bytes()).expect("Writing public key to pipe failed!");
      stdout_handler.flush().expect("Flushing stdout failed!");

      let msg_to_sign = average.to_le_bytes();

      let signature: Signature = signing_key.sign(&msg_to_sign); 
      stdout_handler.write_all(signature.to_bytes().as_slice()).expect("Writing signature to pipe failed!");
      stdout_handler.flush().expect("Flushing stdout failed!");

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
              "cd {} && ./target/debug/SupraOracles_Rust_3 --mode=cache --times=10 --kickoff={} 2>&1", 
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
        let mut price: String = String::new();

        let mut vk_from_child: [u8; 65] = [0u8; 65];
        
        match child.stdout.as_mut().unwrap().read_exact(&mut vk_from_child) {
          Err(why) => panic!("couldn't read stdout: {}", why),
          Ok(_) => {
            // println!("Public key: {:x?}", vk_from_child);
          },
          }
          let verify_key = VerifyingKey::from_encoded_point(
            &EncodedPoint::from_bytes(&vk_from_child).expect("Conversion from bytes to encoded point unsuccessful!")
          ).expect("Veryfying key cannot be restored after pipe transfer");

          
          let mut signature: [u8; 64] = [0u8; 64];

          match child.stdout.as_mut().unwrap().read_exact(&mut signature) {
            Err(why) => panic!("couldn't read stdout: {}", why),
            Ok(_) => {
              // println!("Signature: {:x?}", signature);
            },
          }

          let recovered_signature = Signature::from_slice(signature.as_slice())
            .expect("Could not recover signature from byte slice");

          match child.stdout.take().unwrap().read_to_string(&mut price) {
              Err(why) => panic!("couldn't read stdout: {}", why),
              Ok(_) => {
                println!("Child responded with the following price: {}", price);
                let parsed_price = price.parse::<f64>().expect("Child process sent a string that is not a float!");
                if let Ok(_) = verify_key.verify(&parsed_price.to_le_bytes(), &recovered_signature) {
                  println!("Signature is valid!");
                  average_of_averages += parsed_price;
                } else {
                  panic!("Signature is not valid!");
                }
              },``
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
