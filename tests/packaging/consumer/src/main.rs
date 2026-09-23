//! A downstream consumer of the packaged `manja`, using public paths only.
//! It runs offline: nothing here opens a connection.

#[cfg(feature = "decoder")]
fn decoder() {
    use manja::kite::decoder::framing::{frame, FramingLimits, Message};
    use manja::kite::decoder::packets::{decode, Packet};
    use manja::kite::decoder::text::{parse, TextEvent, TextLimits};
    // One LTP packet: token 256265, last price 2245000.
    let bytes = [0, 1, 0, 8, 0, 3, 0xE9, 0x09, 0, 0x22, 0x41, 0x88];
    let Ok(Message::Packets(frames)) = frame(&bytes, FramingLimits::default()) else {
        panic!("frames")
    };
    let packet = decode(&frames.iter().next().unwrap()).unwrap();
    assert!(matches!(packet, Packet::Ltp(p) if p.instrument_token.get() == 256265 && p.last_price == 2245000));
    assert!(matches!(
        parse(r#"{"type":"message","data":"hi"}"#, TextLimits::default()),
        Ok(TextEvent::Message(_))
    ));
    println!("decoder: ok");
}

#[cfg(feature = "http")]
fn http() {
    use manja::kite::connect::client::HTTPClient;
    use manja::kite::connect::config::Config;
    use manja::kite::connect::credentials::{AccessToken, ApiKey, ApiSecret, Credentials, RequestToken};
    // The required session operations exist with `http` alone.
    let client = HTTPClient::new(Config::new("http://127.0.0.1:1")).unwrap();
    let session = client.session(ApiKey::new("key").unwrap());
    let (request, secret) = (
        RequestToken::new("request").unwrap(),
        ApiSecret::new("secret").unwrap(),
    );
    let token = AccessToken::new("token").unwrap();
    let exchange = session.exchange(&request, &secret);
    let invalidate = session.invalidate(&token);
    // Futures are lazy: dropping them unpolled sends nothing.
    drop((exchange, invalidate));
    let authed = client.with_credentials(Credentials::new("key", "token").unwrap());
    let _ = authed.portfolio();
    println!("http: ok");
}

#[cfg(feature = "ticker")]
fn ticker() {
    use manja::kite::connect::credentials::Credentials;
    use manja::kite::ticker::actor::owner::{TickerBuilder, TickerSpawnError};
    // Outside a runtime a ticker refuses to start, explicitly.
    let err = TickerBuilder::new(Credentials::new("key", "token").unwrap())
        .spawn()
        .unwrap_err();
    assert_eq!(err, TickerSpawnError::NoRuntime);
    println!("ticker: ok");
}

fn main() {
    #[cfg(feature = "decoder")]
    decoder();
    #[cfg(feature = "http")]
    http();
    #[cfg(feature = "ticker")]
    ticker();
    let _ = manja::kite::protocol::InstrumentToken::new(1);
    println!("consumer: ok");
}
