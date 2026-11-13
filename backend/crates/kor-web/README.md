# kor-web

**What it does.** The webmaster and agency module (spec §29): hosting panels, WordPress/WooCommerce, technical SEO audit, Search Console/Analytics, content pipeline.

**What it does not do.** Nothing runs against a site without the user's configured credentials and approval.

**Called by:** `kor-cli`, `kor-desktop`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).

## Turkish integrations (§29.14) and backlinks (§29.10, §29.16)

Credentials are vault names in `business.toml` (`kor vault add <NAME>`); values never reach logs or error messages.

| Integration | Command | API (public documentation) | Scope |
| --- | --- | --- | --- |
| Param POS | `kor tr payment param` | TurkPos SOAP (`SHA2B64`, `TP_WMD_UCD`, `TP_WMD_Pay`), dev.param.com.tr | PHP template, 3D model; charged only after the 3D check |
| Sipay | `kor tr payment sipay` | `/ccpayment/api/token`, `paySmart3D`, `checkstatus`, apidocs.sipay.com.tr | PHP template; result confirmed server-to-server |
| N11 | `kor tr market` (`[n11] key/secret`) | `ProductService` / `OrderService` SOAP, `pagingData/totalCount` | read-only counts |
| Yurtiçi Kargo | `kor tr cargo <cargo key>` (`[yurtici]`; `id = "test"` for the test service) | `ShippingOrderDispatcherServices.queryShipment` | read-only status |
| Domain Name API | `kor tr dns <domain> [--set "name TYPE value" \| --ns a,b]` (`[dna] id/key`) | REST v1 (`domains/info`, `domains/zones`, `domains/dns/name-server`) as in the vendor's client `domainreseller/php-dna` | read; changes after a yes |
| OpenCart | `kor seo meta opencart <csv> --prefix oc_ --language 1 --oc-version 3\|4` | database (`*_description`, `seo_url`) | SQL apply + undo scripts; old values kept in `<prefix>kor_meta_undo` |
| Ahrefs / Semrush / Moz | `kor seo backlinks <domain>…` | Ahrefs v3 `site-explorer`, Semrush `backlinks_overview`, Moz Links v2 `url_metrics` | read-only; keys `AHREFS_API_KEY`, `SEMRUSH_API_KEY`, `MOZ_ACCESS_ID` + `MOZ_SECRET_KEY` |
| SERP competition | `kor seo competition "<keyword>" --serp <url>…` | the ranking pages (robots-respecting) + backlink authority when connected | deterministic difficulty; formula printed |

Not covered, because the carriers give their API documents to contract customers only: Aras, MNG, Sürat and PTT cargo. Other Turkish registrars publish no API reference; their panels are handled through panel maps (§29.2). The Sipay hash field order and Param's `TP_WMD_UCD` field set follow the public pages; compare them with the provider's test panel before going live (the templates say so).
