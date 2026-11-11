//! UBL-TR 1.2 documents: e-Fatura (TEMELFATURA/TICARIFATURA), e-Arşiv (EARSIVFATURA) invoices and
//! e-İrsaliye despatch advices, with VKN/TCKN checksums and KDV totals in integer kuruş.
//!
//! Kor writes the unsigned XML. Signing (mali mühür) and delivery to GİB go through the user's
//! special integrator (özel entegratör): [`send`] posts the XML to the integrator endpoint the user
//! configured, with a token from the vault. Each integrator has its own API contract; that last
//! step needs the user's integrator account.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::kdv;
use crate::site::Result;
use crate::site::http::Http;

/// Vergi kimlik numarası (10 digits) checksum.
pub fn valid_vkn(v: &str) -> bool {
    let d: Vec<u32> = v.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() != 10 || v.len() != 10 {
        return false;
    }
    let mut sum = 0;
    for (i, digit) in d.iter().take(9).enumerate() {
        let i = u32::try_from(i).unwrap_or(0);
        let v1 = (digit + 9 - i) % 10;
        let mut v2 = (v1 * 2u32.pow(9 - i)) % 9;
        if v1 != 0 && v2 == 0 {
            v2 = 9;
        }
        sum += v2;
    }
    (10 - sum % 10) % 10 == d[9]
}

/// T.C. kimlik numarası (11 digits) checksum.
pub fn valid_tckn(v: &str) -> bool {
    let d: Vec<u32> = v.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() != 11 || v.len() != 11 || d[0] == 0 {
        return false;
    }
    let odd = d[0] + d[2] + d[4] + d[6] + d[8];
    let even = d[1] + d[3] + d[5] + d[7];
    (odd * 7 + 10 * 10 - even) % 10 == d[9] && d[..10].iter().sum::<u32>() % 10 == d[10]
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Party {
    pub name: String,
    /// VKN (10) or TCKN (11 digits).
    pub tax_id: String,
    #[serde(default)]
    pub tax_office: String,
    #[serde(default)]
    pub street: String,
    /// İlçe.
    #[serde(default)]
    pub district: String,
    /// İl.
    pub city: String,
    #[serde(default)]
    pub email: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    /// Quantity in thousandths (1 piece = 1000).
    pub qty_milli: i64,
    /// UN/ECE unit code: `C62` piece, `HUR` hour, `KGM` kg, `MTR` metre.
    pub unit: String,
    /// Unit price without KDV, kuruş.
    pub price: i64,
    /// KDV percent.
    pub kdv: u32,
}

impl Line {
    /// Net line amount in kuruş, rounded half up.
    pub fn net(&self) -> i64 {
        (self.price * self.qty_milli + 500).div_euclid(1000)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Profile {
    #[default]
    Temel,
    Ticari,
    EArsiv,
}

impl Profile {
    fn id(self) -> &'static str {
        match self {
            Profile::Temel => "TEMELFATURA",
            Profile::Ticari => "TICARIFATURA",
            Profile::EArsiv => "EARSIVFATURA",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Invoice {
    /// `ABC2026000000001`: three letters/digits, the year, nine digits.
    pub id: String,
    pub profile: Profile,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `HH:MM:SS`.
    pub time: String,
    pub supplier: Party,
    pub customer: Party,
    pub lines: Vec<Line>,
    #[serde(default)]
    pub notes: Vec<String>,
}

fn esc(s: &str) -> String {
    crate::site::esc(s)
}

/// Kuruş → `123.45`.
pub fn amount(k: i64) -> String {
    format!("{}{}.{:02}", if k < 0 { "-" } else { "" }, k.unsigned_abs() / 100, k.unsigned_abs() % 100)
}

fn qty(m: i64) -> String {
    let s = format!("{}.{:03}", m / 1000, (m % 1000).abs());
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Random RFC 4122 version 4 UUID.
pub fn uuid4() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().fold(String::new(), |mut o, x| {
        let _ = write!(o, "{x:02x}");
        o
    });
    Ok(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

pub fn valid_doc_id(id: &str) -> bool {
    id.len() == 16
        && id[..3].bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && id[3..].bytes().all(|b| b.is_ascii_digit())
}

fn check_party(p: &Party, role: &str) -> Result<()> {
    if p.name.trim().is_empty() || p.city.trim().is_empty() {
        return Err(format!("{role}: name and city are required"));
    }
    if !(valid_vkn(&p.tax_id) || valid_tckn(&p.tax_id)) {
        return Err(format!("{role}: `{}` is not a valid VKN or TCKN", p.tax_id));
    }
    Ok(())
}

fn party_xml(o: &mut String, p: &Party) {
    let scheme = if p.tax_id.len() == 11 { "TCKN" } else { "VKN" };
    let _ = write!(
        o,
        "<cac:Party><cac:PartyIdentification><cbc:ID schemeID=\"{scheme}\">{}</cbc:ID></cac:PartyIdentification>\
<cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName>\
<cac:PostalAddress><cbc:StreetName>{}</cbc:StreetName><cbc:CitySubdivisionName>{}</cbc:CitySubdivisionName><cbc:CityName>{}</cbc:CityName><cac:Country><cbc:Name>Türkiye</cbc:Name></cac:Country></cac:PostalAddress>\
<cac:PartyTaxScheme><cac:TaxScheme><cbc:Name>{}</cbc:Name></cac:TaxScheme></cac:PartyTaxScheme>",
        esc(&p.tax_id),
        esc(&p.name),
        esc(&p.street),
        esc(&p.district),
        esc(&p.city),
        esc(&p.tax_office)
    );
    if !p.email.is_empty() {
        let _ = write!(o, "<cac:Contact><cbc:ElectronicMail>{}</cbc:ElectronicMail></cac:Contact>", esc(&p.email));
    }
    if scheme == "TCKN" {
        let (first, family) = p.name.rsplit_once(' ').unwrap_or((&p.name, ""));
        let _ = write!(o, "<cac:Person><cbc:FirstName>{}</cbc:FirstName><cbc:FamilyName>{}</cbc:FamilyName></cac:Person>", esc(first), esc(family));
    }
    o.push_str("</cac:Party>");
}

fn tax_total(o: &mut String, groups: &[(u32, i64, i64)]) {
    let total: i64 = groups.iter().map(|g| g.2).sum();
    let _ = write!(o, "<cac:TaxTotal><cbc:TaxAmount currencyID=\"TRY\">{}</cbc:TaxAmount>", amount(total));
    for (rate, base, tax) in groups {
        let _ = write!(
            o,
            "<cac:TaxSubtotal><cbc:TaxableAmount currencyID=\"TRY\">{}</cbc:TaxableAmount><cbc:TaxAmount currencyID=\"TRY\">{}</cbc:TaxAmount>\
<cbc:Percent>{rate}</cbc:Percent><cac:TaxCategory><cac:TaxScheme><cbc:Name>KDV</cbc:Name><cbc:TaxTypeCode>0015</cbc:TaxTypeCode></cac:TaxScheme></cac:TaxCategory></cac:TaxSubtotal>",
            amount(*base),
            amount(*tax)
        );
    }
    o.push_str("</cac:TaxTotal>");
}

/// Totals: (net, KDV, gross) and per-rate groups (rate, base, tax).
pub fn totals(lines: &[Line]) -> (i64, i64, i64, Vec<(u32, i64, i64)>) {
    let mut groups: Vec<(u32, i64, i64)> = Vec::new();
    for l in lines {
        let net = l.net();
        let (tax, _) = kdv(net, l.kdv);
        match groups.iter_mut().find(|g| g.0 == l.kdv) {
            Some(g) => {
                g.1 += net;
                g.2 += tax;
            }
            None => groups.push((l.kdv, net, tax)),
        }
    }
    let net: i64 = groups.iter().map(|g| g.1).sum();
    let tax: i64 = groups.iter().map(|g| g.2).sum();
    (net, tax, net + tax, groups)
}

const NS: &str = "xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:{DOC}-2\" \
xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\" \
xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\"";

/// The invoice as UBL-TR 1.2 XML (unsigned).
pub fn invoice_xml(inv: &Invoice, uuid: &str) -> Result<String> {
    if !valid_doc_id(&inv.id) {
        return Err(format!("invoice number `{}` must look like ABC2026000000001", inv.id));
    }
    check_party(&inv.supplier, "supplier")?;
    check_party(&inv.customer, "customer")?;
    if inv.lines.is_empty() {
        return Err("an invoice needs at least one line".into());
    }
    if let Some(l) = inv.lines.iter().find(|l| !super::KDV_RATES.contains(&l.kdv) && l.kdv != 0) {
        return Err(format!("{}: KDV {}% is not a current rate (0, 1, 10, 20)", l.name, l.kdv));
    }
    let (net, _tax, gross, groups) = totals(&inv.lines);
    let mut o = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Invoice {}>", NS.replace("{DOC}", "Invoice"));
    let _ = write!(
        o,
        "<cbc:UBLVersionID>2.1</cbc:UBLVersionID><cbc:CustomizationID>TR1.2</cbc:CustomizationID><cbc:ProfileID>{}</cbc:ProfileID>\
<cbc:ID>{}</cbc:ID><cbc:CopyIndicator>false</cbc:CopyIndicator><cbc:UUID>{uuid}</cbc:UUID><cbc:IssueDate>{}</cbc:IssueDate><cbc:IssueTime>{}</cbc:IssueTime>\
<cbc:InvoiceTypeCode>SATIS</cbc:InvoiceTypeCode>",
        inv.profile.id(),
        esc(&inv.id),
        esc(&inv.date),
        esc(&inv.time)
    );
    for n in &inv.notes {
        let _ = write!(o, "<cbc:Note>{}</cbc:Note>", esc(n));
    }
    let _ = write!(o, "<cbc:DocumentCurrencyCode>TRY</cbc:DocumentCurrencyCode><cbc:LineCountNumeric>{}</cbc:LineCountNumeric>", inv.lines.len());
    o.push_str("<cac:AccountingSupplierParty>");
    party_xml(&mut o, &inv.supplier);
    o.push_str("</cac:AccountingSupplierParty><cac:AccountingCustomerParty>");
    party_xml(&mut o, &inv.customer);
    o.push_str("</cac:AccountingCustomerParty>");
    tax_total(&mut o, &groups);
    let _ = write!(
        o,
        "<cac:LegalMonetaryTotal><cbc:LineExtensionAmount currencyID=\"TRY\">{n}</cbc:LineExtensionAmount><cbc:TaxExclusiveAmount currencyID=\"TRY\">{n}</cbc:TaxExclusiveAmount>\
<cbc:TaxInclusiveAmount currencyID=\"TRY\">{g}</cbc:TaxInclusiveAmount><cbc:PayableAmount currencyID=\"TRY\">{g}</cbc:PayableAmount></cac:LegalMonetaryTotal>",
        n = amount(net),
        g = amount(gross)
    );
    for (i, l) in inv.lines.iter().enumerate() {
        let lnet = l.net();
        let (ltax, _) = kdv(lnet, l.kdv);
        let _ = write!(
            o,
            "<cac:InvoiceLine><cbc:ID>{}</cbc:ID><cbc:InvoicedQuantity unitCode=\"{}\">{}</cbc:InvoicedQuantity><cbc:LineExtensionAmount currencyID=\"TRY\">{}</cbc:LineExtensionAmount>",
            i + 1,
            esc(&l.unit),
            qty(l.qty_milli),
            amount(lnet)
        );
        tax_total(&mut o, &[(l.kdv, lnet, ltax)]);
        let _ = write!(
            o,
            "<cac:Item><cbc:Name>{}</cbc:Name></cac:Item><cac:Price><cbc:PriceAmount currencyID=\"TRY\">{}</cbc:PriceAmount></cac:Price></cac:InvoiceLine>",
            esc(&l.name),
            amount(l.price)
        );
    }
    o.push_str("</Invoice>\n");
    Ok(o)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Despatch {
    pub id: String,
    pub date: String,
    pub time: String,
    pub supplier: Party,
    pub customer: Party,
    /// Driver name and TCKN, or the carrier's VKN and name.
    #[serde(default)]
    pub driver: Option<(String, String)>,
    #[serde(default)]
    pub carrier: Option<(String, String)>,
    #[serde(default)]
    pub plate: Option<String>,
    pub lines: Vec<Line>,
}

/// e-İrsaliye (UBL-TR DespatchAdvice, TEMELIRSALIYE, SEVK).
pub fn despatch_xml(d: &Despatch, uuid: &str) -> Result<String> {
    if !valid_doc_id(&d.id) {
        return Err(format!("despatch number `{}` must look like ABC2026000000001", d.id));
    }
    check_party(&d.supplier, "supplier")?;
    check_party(&d.customer, "customer")?;
    if d.driver.is_none() && d.carrier.is_none() {
        return Err("an e-İrsaliye needs a driver (name + TCKN) or a carrier (VKN + name)".into());
    }
    let mut o = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DespatchAdvice {}>", NS.replace("{DOC}", "DespatchAdvice"));
    let _ = write!(
        o,
        "<cbc:UBLVersionID>2.1</cbc:UBLVersionID><cbc:CustomizationID>TR1.2.1</cbc:CustomizationID><cbc:ProfileID>TEMELIRSALIYE</cbc:ProfileID>\
<cbc:ID>{}</cbc:ID><cbc:CopyIndicator>false</cbc:CopyIndicator><cbc:UUID>{uuid}</cbc:UUID><cbc:IssueDate>{}</cbc:IssueDate><cbc:IssueTime>{}</cbc:IssueTime>\
<cbc:DespatchAdviceTypeCode>SEVK</cbc:DespatchAdviceTypeCode><cbc:LineCountNumeric>{}</cbc:LineCountNumeric>",
        esc(&d.id),
        esc(&d.date),
        esc(&d.time),
        d.lines.len()
    );
    o.push_str("<cac:DespatchSupplierParty>");
    party_xml(&mut o, &d.supplier);
    o.push_str("</cac:DespatchSupplierParty><cac:DeliveryCustomerParty>");
    party_xml(&mut o, &d.customer);
    o.push_str("</cac:DeliveryCustomerParty><cac:Shipment><cbc:ID>1</cbc:ID><cac:ShipmentStage>");
    if let Some((name, tckn)) = &d.driver {
        if !valid_tckn(tckn) {
            return Err("driver TCKN is not valid".into());
        }
        let (first, family) = name.rsplit_once(' ').unwrap_or((name, ""));
        let _ = write!(o, "<cac:DriverPerson><cbc:FirstName>{}</cbc:FirstName><cbc:FamilyName>{}</cbc:FamilyName><cbc:Title>Şoför</cbc:Title><cbc:NationalityID>{}</cbc:NationalityID></cac:DriverPerson>", esc(first), esc(family), esc(tckn));
    }
    o.push_str("</cac:ShipmentStage><cac:Delivery>");
    if let Some((vkn, name)) = &d.carrier {
        let _ = write!(o, "<cac:CarrierParty><cac:PartyIdentification><cbc:ID schemeID=\"VKN\">{}</cbc:ID></cac:PartyIdentification><cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName></cac:CarrierParty>", esc(vkn), esc(name));
    }
    let _ = write!(o, "<cac:Despatch><cbc:ActualDespatchDate>{}</cbc:ActualDespatchDate><cbc:ActualDespatchTime>{}</cbc:ActualDespatchTime></cac:Despatch>", esc(&d.date), esc(&d.time));
    o.push_str("</cac:Delivery>");
    if let Some(p) = &d.plate {
        let _ = write!(o, "<cac:TransportHandlingUnit><cac:TransportEquipment><cbc:ID schemeID=\"PLAKA\">{}</cbc:ID></cac:TransportEquipment></cac:TransportHandlingUnit>", esc(p));
    }
    o.push_str("</cac:Shipment>");
    for (i, l) in d.lines.iter().enumerate() {
        let _ = write!(
            o,
            "<cac:DespatchLine><cbc:ID>{n}</cbc:ID><cbc:DeliveredQuantity unitCode=\"{}\">{}</cbc:DeliveredQuantity><cac:OrderLineReference><cbc:LineID>{n}</cbc:LineID></cac:OrderLineReference><cac:Item><cbc:Name>{}</cbc:Name></cac:Item></cac:DespatchLine>",
            esc(&l.unit),
            qty(l.qty_milli),
            esc(&l.name),
            n = i + 1
        );
    }
    o.push_str("</DespatchAdvice>\n");
    Ok(o)
}

/// Post the XML to the user's integrator (`url` from `business.toml`, token from the vault).
pub async fn send(http: &Http, url: &str, token: &str, xml: &str) -> Result<String> {
    let r = http
        .send(
            reqwest::Method::POST,
            url,
            &[("authorization", format!("Bearer {token}"))],
            Some(("application/xml; charset=utf-8", xml.as_bytes().to_vec())),
        )
        .await?;
    if r.ok() {
        Ok(r.body.chars().take(2000).collect())
    } else {
        Err(format!("integrator: HTTP {}: {}", r.status, r.body.chars().take(500).collect::<String>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VKN with a correct check digit for the first nine digits.
    fn make_vkn(first9: &str) -> String {
        (0..10).map(|c| format!("{first9}{c}")).find(|v| valid_vkn(v)).unwrap()
    }

    fn party(name: &str, id: &str) -> Party {
        Party { name: name.into(), tax_id: id.into(), tax_office: "Kadıköy".into(), city: "İstanbul".into(), ..Party::default() }
    }

    #[test]
    fn identity_numbers() {
        assert!(valid_tckn("10000000146"));
        assert!(!valid_tckn("10000000147") && !valid_tckn("00000000146"));
        let v = make_vkn("123456789");
        assert!(valid_vkn(&v));
        let mut bad = v.clone();
        bad.replace_range(9..10, if v.ends_with('0') { "1" } else { "0" });
        assert!(!valid_vkn(&bad));
        assert!(valid_doc_id("KOR2026000000001") && !valid_doc_id("KOR26000000001"));
    }

    #[test]
    fn invoice_totals_and_xml() {
        let inv = Invoice {
            id: "KOR2026000000001".into(),
            profile: Profile::EArsiv,
            date: "2026-11-10".into(),
            time: "10:00:00".into(),
            supplier: party("Acme Yazılım <Ltd>", &make_vkn("123456789")),
            customer: party("Ayşe Yılmaz", "10000000146"),
            lines: vec![
                Line { name: "Web sitesi bakımı".into(), qty_milli: 1500, unit: "HUR".into(), price: 100_000, kdv: 20 },
                Line { name: "Kitap".into(), qty_milli: 1000, unit: "C62".into(), price: 5_000, kdv: 10 },
            ],
            notes: vec![],
        };
        let (net, tax, gross, groups) = totals(&inv.lines);
        assert_eq!((net, tax, gross), (155_000, 30_500, 185_500));
        assert_eq!(groups.len(), 2);
        let xml = invoice_xml(&inv, "uuid-1").unwrap();
        assert!(xml.contains("<cbc:ProfileID>EARSIVFATURA</cbc:ProfileID>"));
        assert!(xml.contains("<cbc:PayableAmount currencyID=\"TRY\">1855.00</cbc:PayableAmount>"));
        assert!(xml.contains("<cbc:InvoicedQuantity unitCode=\"HUR\">1.5</cbc:InvoicedQuantity>"));
        assert!(xml.contains("schemeID=\"TCKN\">10000000146") && xml.contains("<cbc:FamilyName>Yılmaz</cbc:FamilyName>"));
        assert!(xml.contains("Acme Yazılım &lt;Ltd&gt;"));
        let bad = Invoice { customer: party("X", "123"), ..inv.clone() };
        assert!(invoice_xml(&bad, "u").is_err());
        let d = Despatch {
            id: "KOR2026000000002".into(),
            date: "2026-11-10".into(),
            time: "11:00:00".into(),
            supplier: inv.supplier.clone(),
            customer: inv.customer.clone(),
            driver: Some(("Ali Veli".into(), "10000000146".into())),
            carrier: None,
            plate: Some("34ABC123".into()),
            lines: inv.lines.clone(),
        };
        let x = despatch_xml(&d, "u2").unwrap();
        assert!(x.contains("<cbc:DespatchAdviceTypeCode>SEVK</cbc:DespatchAdviceTypeCode>") && x.contains("PLAKA"));
        let u = uuid4().unwrap();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
    }
}
