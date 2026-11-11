//! Marketplace adapters (read-only): Trendyol seller API and Hepsiburada listing API. Credentials
//! are vault names in `business.toml`; Kor reads counts only and never changes listings or orders.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::site::Result;
use crate::site::http::Http;

pub const TRENDYOL: &str = "https://apigw.trendyol.com";
pub const HEPSIBURADA: &str = "https://listing-external.hepsiburada.com";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counts {
    pub market: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub products: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orders: Option<u64>,
}

fn basic(user: &str, pass: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}")))
}

fn digits(s: &str) -> Result<()> {
    if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(format!("invalid seller/merchant id `{s}`"))
    }
}

/// Trendyol: product and order totals (`totalElements`). The API wants the seller id in the user agent.
pub async fn trendyol(http: &Http, base: &str, seller: &str, key: &str, secret: &str) -> Result<Counts> {
    digits(seller)?;
    let h = [("authorization", basic(key, secret)), ("user-agent", format!("{seller} - SelfIntegration"))];
    Ok(Counts {
        market: "Trendyol".into(),
        products: trendyol_total(http, &format!("{base}/integration/product/sellers/{seller}/products?page=0&size=1"), &h).await?,
        orders: trendyol_total(http, &format!("{base}/integration/order/sellers/{seller}/orders?page=0&size=1"), &h).await?,
    })
}

async fn trendyol_total(http: &Http, url: &str, h: &[(&str, String)]) -> Result<Option<u64>> {
    let r = http.get_with(url, h).await?;
    if !r.ok() {
        return Err(format!("Trendyol: HTTP {}", r.status));
    }
    Ok(r.json()?["totalElements"].as_u64())
}

/// Hepsiburada: listing total (`totalCount`).
pub async fn hepsiburada(http: &Http, base: &str, merchant: &str, user: &str, pass: &str) -> Result<Counts> {
    let ok = !merchant.is_empty() && merchant.len() <= 64 && merchant.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !ok {
        return Err(format!("invalid merchant id `{merchant}`"));
    }
    let r = http
        .get_with(&format!("{base}/listings/merchantid/{merchant}?offset=0&limit=1"), &[("authorization", basic(user, pass))])
        .await?;
    if !r.ok() {
        return Err(format!("Hepsiburada: HTTP {}", r.status));
    }
    Ok(Counts { market: "Hepsiburada".into(), products: r.json()?["totalCount"].as_u64(), orders: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn counts_from_mock_apis() {
        let s = MockServer::start().await;
        Mock::given(path("/integration/product/sellers/123/products")).and(header("user-agent", "123 - SelfIntegration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"totalElements": 42}))).mount(&s).await;
        Mock::given(path("/integration/order/sellers/123/orders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"totalElements": 7}))).mount(&s).await;
        Mock::given(path("/listings/merchantid/m-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"totalCount": 9}))).mount(&s).await;
        let http = Http::new().unwrap();
        let t = trendyol(&http, &s.uri(), "123", "k", "s").await.unwrap();
        assert_eq!((t.products, t.orders), (Some(42), Some(7)));
        assert_eq!(hepsiburada(&http, &s.uri(), "m-1", "u", "p").await.unwrap().products, Some(9));
        assert!(trendyol(&http, &s.uri(), "12/3", "k", "s").await.is_err());
    }
}
