# Çalışma zamanı ortamı — değişkenler, durum, proxy, capture

Kaynak: `RuriLib/Models/Bots/BotData.cs`, `RuriLib/Functions/Conditions/Conditions.cs`,
`RuriLib/Models/Jobs/*`, `RuriLib/Helpers/ConfigPacker.cs`, OB1 `RuriLib/Models/BotData.cs` (miras davranış).

## 1. Yerleşik değişkenler

`data.*` — ayrılmış, `SET VAR` ile ezilemez (yalnızca `SET SOURCE` / `SET RESPONSECODE` gibi
özel `SET` biçimleriyle değişir):

| Değişken | Tip | Anlam |
|---|---|---|
| `data.SOURCE` | string | son HTTP yanıtının gövdesi |
| `data.RESPONSECODE` | int | HTTP durum kodu |
| `data.HEADERS` | dict | yanıt başlıkları |
| `data.COOKIES` | dict | çerez kavanozu — istekler arasında **birikir, silinmez** |
| `data.ADDRESS` | string | son isteğin adresi |
| `data.ERROR` | string | SAFE bloğunun yakaladığı istisna |
| `data.UseProxy` | bool | proxy kullanımı |
| `data.STATUS` | string | bot durumu (SUCCESS/FAIL/BAN/RETRY/CUSTOM) |

`input.*` — Wordlist/custom input alanları: `input.USER`, `input.PASS`, `input.EMAIL`, `input.DOM`,
`input.apikey`, `input.bottoken` … Wordlist bölümlemesinden (`WORDLIST TYPE`) üretilir.

`globals.*` — botlar arası paylaşılan sözlük. **Yarış koşulu vardır**: yazarken `LOCK globals`
(veya `ACQUIRELOCK`) kullan, yoksa sayaç kaybolur.

## 2. Bot durumu

`NONE`'da başlar; yalnızca `NONE` ve `SUCCESS` durumundayken devam eder. Başka duruma geçtiği anda
script biter. `ContinueStatuses` ayarı bir durumu "devam" listesine alabilir (`ContinueStatuses = ["RETRY"]`).

| Durum | Sonuç |
|---|---|
| `SUCCESS` | hit kaydı |
| `FAIL` | geçersiz kullanıcı/şifre |
| `BAN` | proxy banlanır + data tekrar denenir |
| `RETRY` | data tekrar denenir |
| `CUSTOM` | custom kovası (2FA, e-posta doğrulama bekleyen) |
| `ERROR` | istisna: loglanır, sonra BAN gibi davranır |

Global ban/retry anahtarları proxy ayarlarında tanımlıdır ve **her KeyCheck'ten sonra** otomatik kontrol edilir.
Yani kaynak metninde o kelime geçen bir yanıt, yazdığın keychain'i ezebilir.

## 3. Proxy

- Tipler: HTTP, HTTPS, SOCKS4, SOCKS4A, SOCKS5.
- `SET PROXY "host" port SOCKS5 ["user"] ["pass"]`, `SET USEPROXY TRUE|FALSE`.
- `Proxy.BanLoopEvasion` — aynı data satırı kaç kez BAN olabilir; aşılırsa `ToCheck`'e düşer.
- Cloudflare `cf_clearance` çerezi **proxy'ye bağlıdır**: aynı IP olmadan geçerli değildir.
  Bu yüzden CF'li hedeflerde sticky proxy şart; rotasyonlu proxy'de `cf_clearance` işe yaramaz.

## 4. Capture (hit verisi)

- `=> CAP @x` veya `MARK @x` capture işaretler; `SET CAP @x "deger"` doğrudan yazar.
- `SaveEmptyCaptures = false` ise boş capture oluşmaz (hit çıktısında alan görünmez).
- Hit export şablonu `<DATA>, <PROXY>, <DATE>, <CONFIG>, <WORDLIST>, <CAPTURE>, <x>` değişkenlerini kullanır.

## 5. Kaynaklar (Resources)

`TAKE 5 FROM "ad" => @liste` / `TAKEONE FROM "ad" => @tek`. Kaynaklar Config Settings > Data > Resources'ta
tanımlanır (dosya veya API). Botlar arasında atomik tüketilir — API anahtarı havuzu, e-posta havuzu için doğru araç.

## 6. Config paketi (.opk)

`.opk` bir **zip**'tir: `readme.md`, `metadata.json`, `settings.json`, `script.loli`, `startup.loli`
(veya `script.cs` / `build.dll` / `script.legacy`). `settings.json` içinde GeneralSettings, ProxySettings,
InputSettings, DataSettings, BrowserSettings, ScriptSettings bulunur.

`script.loli` doğrudan yazılabilir; `.opk` üretmek için zip'lemek yeterlidir:

```
node tools/opk.mjs pack ./config-klasoru  (veya: tools/opk.mjs unpack x.opk ./cikti)
```

MediaType'lar: `application/x-www-form-urlencoded`, `application/json`, `multipart/form-data`.

## 7. OB1 / SilverBullet farkları (kritik)

- OB1'de özel input öneki **yoktur**: `CustomInput.VariableName` doğrudan `<VARADI>` olur. `input.USER`
  sözdizimi **OB2'ye aittir** — OB1'de aynı betiği çalıştırmaz.
- OB1 Wordlist tipleri `Environment.ini`'deki `[WLTYPE]` ile tanımlanır: `Regex`, `Verify`, `Separator`, `Slices`.
- OB1 `.loli` = `[SETTINGS]` JSON bloğu + `[SCRIPT]` LoliScript gövdesi. LoliScript XML değildir.
- OB1 blok anahtar sözcükleri BÜYÜK harftir (`REQUEST`, `PARSE`, `KEYCHECK`, `FUNCTION`, `UTILITY`).
- OB1'de `KEYCHAIN` yoktur, `KEY` vardır: `KEY "metin"` kısa biçim (SOURCE Contains), `KEY "<sol>" Compare "<sağ>"` uzun biçim.
- OB1'de `EXISTS`/`DOESNOTEXIST` sol terim **atanabildiyse** tetiklenir (boş değil demek değil).
- OB1'de `PARSE` tipe özel ayar adları: `LeftString`, `RightString`, `CssSelector`, `JsonField`, `RegexString`…
  OB2'de bunlar `leftDelim`, `cssSelector`, `jToken`, `pattern` oldu.
- OB1 `REQUEST` ayarları: `Url`, `Method`, `RequestType`, `CustomHeaders`, `CustomCookies`, `ContentType`,
  `AutoRedirect`, `ReadResponseSource`, `EncodeContent`, `SecurityProtocol`, `ResponseType`, `PostData`, `RawData`.
- SilverBullet = OB1 fork'u; aynı `.loli` biçimi, aynı bloklar. Fark: düzeltilmiş keycheck/idle, ek captcha
  servisleri, OCR, gelişmiş DB. Config uyumluluğu yüksektir; `RequiredPlugins` alanı farklı olabilir.
- OB1/SB'de satır yorumu `##`, devre dışı satır `!`; ayrıca satır numarası hataları farklı raporlanır.
