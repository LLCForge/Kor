//! Cargo (shipping) adapters. Only Yurtiçi Kargo publishes its web service contract (the
//! `ShippingOrderDispatcherServices` WSDL with public test credentials); Kor queries a shipment's
//! status there, read-only. Aras, MNG, Sürat and PTT hand their API documents to contract
//! customers only, so they have no adapter (see `docs/webmaster.md`).

use serde::Serialize;

use crate::site::Result;
use crate::site::esc;
use crate::site::http::Http;

pub const YURTICI: &str =
    "https://ws.yurticikargo.com/KOPSWebServices/ShippingOrderDispatcherServices";
pub const YURTICI_TEST: &str =
    "http://testwebservices.yurticikargo.com:9090/KOPSWebServices/ShippingOrderDispatcherServices";

/// The status of one shipment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Shipment {
    pub key: String,
    /// Carrier code, e.g. Yurtiçi `NOP` (not processed), `IND` (in delivery), `DLV` (delivered).
    pub status: String,
    pub message: String,
    /// Carrier tracking number once the parcel is accepted.
    pub tracking: String,
}

/// Text of the first `<name>` or `<prefix:name>` element (entities decoded), if any.
pub(crate) fn xml_tag(xml: &str, name: &str) -> Option<String> {
    let mut rest = xml;
    loop {
        let i = rest.find('<')?;
        rest = &rest[i + 1..];
        let end = rest.find('>')?;
        let open = &rest[..end];
        let local = open.split_whitespace().next()?;
        let local = local.rsplit(':').next()?;
        if local == name && !open.starts_with('/') && !open.ends_with('/') {
            let body = &rest[end + 1..];
            let close = body.find("</")?;
            return Some(
                body[..close]
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'")
                    .replace("&amp;", "&")
                    .trim()
                    .to_owned(),
            );
        }
    }
}

/// `queryShipment` SOAP body for one cargo key.
fn query_body(user: &str, pass: &str, key: &str) -> String {
    format!(
        "<soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         xmlns:ship=\"http://yurticikargo.com.tr/ShippingOrderDispatcherServices\">\
         <soapenv:Header/><soapenv:Body><ship:queryShipment>\
         <wsUserName>{}</wsUserName><wsPassword>{}</wsPassword><wsLanguage>TR</wsLanguage>\
         <keys>{}</keys><keyType>0</keyType><addHistoricalData>false</addHistoricalData>\
         <onlyTracking>false</onlyTracking></ship:queryShipment></soapenv:Body></soapenv:Envelope>",
        esc(user),
        esc(pass),
        esc(key)
    )
}

/// Yurtiçi Kargo: status of the shipment with this cargo key (the merchant's own key).
pub async fn yurtici(
    http: &Http,
    endpoint: &str,
    user: &str,
    pass: &str,
    key: &str,
) -> Result<Shipment> {
    if key.is_empty() || key.len() > 64 || key.chars().any(char::is_control) {
        return Err(format!("invalid cargo key `{key}`"));
    }
    let r = http
        .send(
            reqwest::Method::POST,
            endpoint,
            &[("soapaction", "\"\"".into())],
            Some((
                "text/xml; charset=utf-8",
                query_body(user, pass, key).into_bytes(),
            )),
        )
        .await?;
    if let Some(fault) = xml_tag(&r.body, "faultstring") {
        return Err(format!("Yurtiçi: {fault}"));
    }
    if !r.ok() {
        return Err(format!("Yurtiçi: HTTP {}", r.status));
    }
    // outFlag 0 = success; anything else carries outResult / errMessage.
    if xml_tag(&r.body, "outFlag").is_some_and(|f| f != "0") {
        let why = xml_tag(&r.body, "errMessage")
            .or_else(|| xml_tag(&r.body, "outResult"))
            .unwrap_or_default();
        return Err(format!("Yurtiçi: {why}"));
    }
    Ok(Shipment {
        key: key.to_owned(),
        status: xml_tag(&r.body, "operationStatus").unwrap_or_default(),
        message: xml_tag(&r.body, "operationMessage").unwrap_or_default(),
        tracking: xml_tag(&r.body, "docId").unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const OK: &str = r#"<S:Envelope xmlns:S="http://schemas.xmlsoap.org/soap/envelope/"><S:Body>
<ns2:queryShipmentResponse xmlns:ns2="http://yurticikargo.com.tr/ShippingOrderDispatcherServices">
<ShippingDeliveryVO><outFlag>0</outFlag><outResult>Başarılı</outResult><count>1</count>
<shippingDeliveryDetailVO><cargoKey>K1</cargoKey><operationStatus>IND</operationStatus>
<operationMessage>Kargo teslimatta</operationMessage><docId>123456789</docId></shippingDeliveryDetailVO>
</ShippingDeliveryVO></ns2:queryShipmentResponse></S:Body></S:Envelope>"#;

    #[tokio::test]
    async fn yurtici_status_from_mock() {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("<keys>K1</keys>"))
            .and(body_string_contains("<wsUserName>YKTEST</wsUserName>"))
            .respond_with(ResponseTemplate::new(200).set_body_string(OK))
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("<keys>K2</keys>"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<x><outFlag>1</outFlag><errMessage>Kayıt bulunamadı</errMessage></x>",
            ))
            .mount(&s)
            .await;
        let http = Http::new().unwrap();
        let r = yurtici(&http, &s.uri(), "YKTEST", "YK", "K1")
            .await
            .unwrap();
        assert_eq!(
            (r.status.as_str(), r.tracking.as_str()),
            ("IND", "123456789")
        );
        let e = yurtici(&http, &s.uri(), "YKTEST", "YK", "K2")
            .await
            .unwrap_err();
        assert!(e.contains("Kayıt bulunamadı"));
        assert!(yurtici(&http, &s.uri(), "u", "p", "").await.is_err());
        // Credentials are escaped into the envelope, never interpreted as XML.
        assert!(query_body("a<b", "p", "k").contains("a&lt;b"));
        assert_eq!(
            xml_tag("<a:x y=\"1\">v&amp;w</a:x>", "x").as_deref(),
            Some("v&w")
        );
    }
}
