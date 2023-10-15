use crate::{error::Error, record::Record};
use async_trait::async_trait;
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

#[derive(Clone, Copy, Debug, Display, Deserialize, EnumString, PartialEq, Serialize)]
pub enum Symbol {
    #[serde(alias = "1INCHBTC", alias = "1INCH-BTC", alias = "1inchbtc")]
    #[strum(to_string = "1inch-btc")]
    Oneinchbtc,
    #[serde(alias = "BCHUSDT", alias = "BCH-USDT", alias = "bchusdt")]
    #[strum(to_string = "bch-usdt")]
    Bchusdt,
    #[serde(alias = "BNBUSDT", alias = "BNB-USDT", alias = "bnbusdt")]
    #[strum(to_string = "bnb-usdt")]
    Bnbusdt,
    #[serde(alias = "BTCEUR", alias = "BTC-EUR", alias = "btceur")]
    #[strum(to_string = "btc-eur")]
    Btceur,
    #[serde(alias = "BTCUSDT", alias = "BTC-USDT", alias = "btcusdt")]
    #[strum(to_string = "btc-usdt")]
    Btcusdt,
    #[serde(alias = "BTCUSD", alias = "BTC-USD", alias = "btcusd")]
    #[strum(to_string = "btc-usd")]
    Btcusd,
    #[serde(alias = "BTCTUSD", alias = "BTCT-USD", alias = "btctusd")]
    #[strum(to_string = "btct-usd")]
    Btctusd,
    #[serde(alias = "BUSDUSDT", alias = "BUSD-USDT", alias = "busdusdt")]
    #[strum(to_string = "busd-usdt")]
    Busdusdt,
    #[serde(alias = "CBETHETH", alias = "CBETH-ETH", alias = "cbetheth")]
    #[strum(to_string = "cbeth-eth")]
    Cbetheth,
    #[serde(alias = "DOGEUSDT", alias = "DOGE-USDT", alias = "dogeusdt")]
    #[strum(to_string = "doge-usdt")]
    Dogeusdt,
    #[serde(alias = "ETHBTC", alias = "ETH-BTC", alias = "ethbtc")]
    #[strum(to_string = "eth-btc")]
    Ethbtc,
    #[serde(alias = "ETHUSD", alias = "ETH-USD", alias = "ethusd")]
    #[strum(to_string = "eth-usd")]
    Ethusd,
    #[serde(alias = "ETHUSDT", alias = "ETH-USDT", alias = "ethusdt")]
    #[strum(to_string = "eth-usdt")]
    Ethusdt,
    #[serde(alias = "PNTUSDT", alias = "PNT-USDT", alias = "pntusdt")]
    #[strum(to_string = "pnt-usdt")]
    Pntusdt,
    #[serde(alias = "SOLUSDT", alias = "SOL-USDT", alias = "solusdt")]
    #[strum(to_string = "sol-usdt")]
    Solusdt,
    #[serde(alias = "PONDUSDT", alias = "POND-USDT", alias = "pondusdt")]
    #[strum(to_string = "pond-usdt")]
    Pondusdt,
    #[serde(alias = "XVGUSDT", alias = "XVG-USDT", alias = "xvgusdt")]
    #[strum(to_string = "xvg-usdt")]
    Xvgusdt,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum OrderSide {
    #[serde(alias = "buy", alias = "BUY")]
    Buy,
    #[serde(alias = "sell", alias = "SELL")]
    Sell,
}

pub type ParsingFunction = Box<dyn Fn(&str) -> Result<Record, Error> + Send + Sync>;

#[async_trait]
#[enum_dispatch]
pub trait ExchangeConfig {
    fn builde_parsing(&self) -> ParsingFunction;
    fn ping(&self) -> Option<String>;
    fn subscription(&self) -> Result<String, Error>;
    async fn uri(&self) -> Result<String, Error>;
}
