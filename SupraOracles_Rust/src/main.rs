use std::{fs::{read_to_string, File}, io::{Read, Write}, thread, time::Duration};
use serde::{Deserialize, Serialize};
use clap::Parser;
use url::Url;
use tungstenite::{connect, stream::MaybeTlsStream, Error, Message, WebSocket};

#[derive(Parser, Debug)]
struct Args {
  #[arg(long)]
  mode: String,
  #[arg(long, value_parser = clap::value_parser!(u16).range(1..))] // protect against division by 0
  times: Option<u16>
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
  // println!("Received: {}", msg);
  Result::Ok(msg.to_string())
}

fn main() {
  let args = Args::parse();
  let mode = args.mode;
  

  match &mode[..] {
    "cache" => {
      let seconds = args.times.expect("In --cache mode, you should provide the number of times you want to query the Binance WS API.");
      let (mut socket, _response) = connect(
          Url::parse("wss://ws-api.binance.com:443/ws-api/v3").unwrap()
      ).expect("Can't connect");
      println!("Connected to a websocket!");
      
      let mut average: f64 = 0.0;
      let mut data_points: Vec<f64> = Vec::new();
      for _ in 0..seconds {
        let api_response = query_binance_api(&mut socket).expect("Error while querying Binance API");
        let parsed_json: Response = serde_json::from_str(&api_response[..])
          .expect("Binance WS API returned an incorrectly formatted JSON string");

        let current_price = parsed_json.result.price.parse::<f64>().expect("Binance WS API returned a price that is not a float");
        println!("Current BTC price in USDT: {:.8}", current_price);
        average += current_price;
        data_points.push(current_price); // don't perform writing to file while fetching data
        thread::sleep(Duration::new(1, 0));
      }
      average /= f64::from(seconds);
      let mut file = File::create("cache.txt").expect("Cache file creation failed!");
      file.write_fmt(format_args!("{:.20} \n", average)).expect("Writing to cache failed.");
      for data_point in data_points {
        file.write_fmt(format_args!("{:.8} \n", data_point)).expect("Writing to cache failed.");
      }
      println!("Cache complete. The average USD price of BTC is: {:.20}", average);
    }
    "read" => {
      for line in read_to_string("cache.txt").unwrap().lines() {
        println!("{}", line.to_string());
      }
    } 
    _ => {
      panic!("Wrong mode value passed!")
    }
  } 
}
