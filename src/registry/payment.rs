use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentRequired {
    pub price_usd_cents: u64,
    pub model: String,
    pub payment_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentSession {
    pub session_jwt: String,
    #[serde(default)]
    pub subscription: bool,
    #[serde(default)]
    pub expires_at: Option<String>,
}

pub fn parse_payment_required(body: &[u8]) -> Result<PaymentRequired, String> {
    serde_json::from_slice(body).map_err(|e| e.to_string())
}

pub fn should_auto_pay(price_usd_cents: u64, threshold_cents: u64) -> bool {
    threshold_cents > 0 && price_usd_cents <= threshold_cents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_402_body_and_auto_pay_threshold() {
        let body = br#"{"price_usd_cents":5,"model":"per-run","payment_endpoint":"https://pay"}"#;
        let parsed = parse_payment_required(body).unwrap();
        assert_eq!(parsed.price_usd_cents, 5);
        assert!(should_auto_pay(5, 10));
        assert!(!should_auto_pay(5, 0));
    }
}
