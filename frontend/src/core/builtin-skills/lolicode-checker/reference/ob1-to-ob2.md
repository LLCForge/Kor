# OB1 / SilverBullet → OB2 geçiş tablosu

Kaynak: OB1 `RuriLib/LoliScript/Parser/BlockParser.cs`, `RuriLib/Blocks/*.cs`; OB2 `RuriLib/Blocks/*`,
`RuriLib/Legacy/LS/*`. Amaç: elinde `.loli` (LoliScript) varsa OB2'ye **kayıpsız** çevirmek.

## 1. Dosya biçimi

| OB1 / SilverBullet | OB2 |
|---|---|
| `[SETTINGS]` JSON + `[SCRIPT]` gövde | `.opk` zip: `script.loli` + `settings.json` + `metadata.json` |
| `.loli` uzantısı, XML değil | OB2 `.loli`'yi içe aktarır ve `.opk`ya çevirir |
| Wordlist tipi `Environment.ini` `[WLTYPE]` | Config Settings > Data > Wordlist Type |
| `##` yorum, `!` devre dışı | `//` yorum, `DISABLED` satırı |
| Hit export `[EXPFORMAT]` | Config Settings > General > Export format |

## 2. Blok eşlemesi

| OB1 / SilverBullet | OB2 | Not |
|---|---|---|
| `REQUEST` | `HttpRequest` | gövde artık `TYPE:STANDARD` + satırlar |
| `PARSE` | `Parse` | `MODE:LR` meta satırı; ayar adları değişti |
| `KEYCHECK` + `KEY` | `Keycheck` + `KEYCHAIN`/`STRINGKEY` | bkz. tablo 4 |
| `FUNCTION Constant` | `ConstantString`/`ConstantInteger`/… | tipe göre ayrıldı |
| `FUNCTION Hash/HMAC` | `HashString`/`HmacString` | |
| `FUNCTION Base64Encode/Decode` | `UTF8ToBase64` / `Base64ToUTF8` | |
| `FUNCTION AESEncrypt/AESDecrypt` | `AESEncrypt`/`AESDecrypt` | |
| `FUNCTION URLEncode/URLDecode` | `UrlEncode`/`UrlDecode` | |
| `FUNCTION GetRandomUA` | `RandomUserAgent` | `platform` ayarı |
| `FUNCTION Delay` | `Delay` | |
| `FUNCTION Compute` | `Compute` | |
| `FUNCTION RandomNum/RandomString` | `RandomInteger`/`RandomString` | |
| `UTILITY List *` | `List Functions` (`AddToList`, `JoinList`, `SortList`, …) | |
| `UTILITY File *` | `Files` (`FileRead`, `FileWrite`, …) | |
| `UTILITY Variable Split` | `Split` | |
| `CAPTCHA` / `SOLVECAPTCHA` | `SolveImageCaptcha` / `SolveTextCaptcha` | |
| `RECAPTCHA` | `SolveRecaptchaV2` / `SolveRecaptchaV3` | |
| `REPORTCAPTCHA` | `ReportLastSolution` | |
| `BYPASSCF` | `SolveCloudflareChallengePage` veya `httpLibrary = CurlImpersonate` | ikincisi ucuz |
| `TCP` | `TcpConnect`/`TcpSend`/`TcpRead`/`TcpDisconnect` | |
| `NAVIGATE` | `BrowserNavigateTo` (veya `SeleniumNavigateTo`) | |
| `ELEMENTACTION` | `BrowserClick`/`BrowserTypeElement`/`BrowserSelect`/… | |
| `BROWSERACTION` | `BrowserOpen`/`BrowserClose`/`BrowserSwitchToTab`/… | |
| `EXECUTEJS` | `BrowserExecuteJs` (veya `SeleniumExecuteJs`) | |

## 3. `PARSE` ayar adları

| OB1 | OB2 |
|---|---|
| `ParseTarget` (`<SOURCE>`) | `input = @data.SOURCE` |
| `LeftString` / `RightString` | `leftDelim` / `rightDelim` |
| `CssSelector` | `cssSelector` |
| `JsonField` | `jToken` |
| `RegexString` | `pattern` |
| `RegexOutput` | `outputFormat` |
| `Type = JSON` | `MODE:Json` (meta satır) |
| `CreateEmpty` | — (her zaman boş string döner) |

## 4. `KEYCHECK` → `Keycheck`

| OB1 | OB2 |
|---|---|
| `KEYCHECK BanOnToCheck=FALSE` | `banIfNoMatch = False` |
| `KEYCHECK BanOn4XX=TRUE` | `RESPONSECODE` üzerinden `INTKEY ... LessThan 400` kuralı |
| `KEYCHAIN Success OR` | `KEYCHAIN SUCCESS OR` |
| `KEYCHAIN Failure OR` | `KEYCHAIN FAIL OR` |
| `KEY "metin"` | `STRINGKEY @data.SOURCE Contains "metin"` |
| `KEY "<sol>" Contains "<sağ>"` | `STRINGKEY "<sol>" Contains "<sağ>"` |
| `KEY` koşul `EXISTS` | `STRINGKEY @x Exists ""` (BOŞ DEĞİL değil; tanımlı olması) |
| `Comparer` (EqualTo, GreaterThan…) | `StrComparison`/`NumComparison` (aynı adlar) |

## 5. Değişken ve sözdizimi farkları

- `<degisken>` interpolasyon her ikisinde de aynı. Ama OB1'de custom input **öneksiz** (`<VARADI>`),
  OB2'de `input.*` (`<input.USER>`).
- OB1'de `SET VAR x v` / `SET CAP x v`; OB2'de `SET VAR @x "v"` (`@` zorunlu, değer tırnaklı).
- OB1'de `-> VAR "ad"`; OB2'de `=> VAR @ad`.
- OB1'de `FUNCTION` 34 fonksiyonu tek blokta topluyordu; OB2'de her şey ayrı blok.
- OB1'de `IF ... ENDIF` / `WHILE ... ENDWHILE`; OB2'de her ikisi de `END` ile kapanır.

## 6. Çeviri sonrası mutlaka kontrol et

1. `KEYCHECK`'lerin **sırası** (OB2'de son eşleşen kazanır — FAIL üstte, SUCCESS altta).
2. `BanOn4XX` yerine yazılan `RESPONSECODE` kuralı.
3. Selenium → Puppeteer geçişinde `ElementLocator` eşlemesi (`class` → `Class`, `xpath` → `XPath`).
4. Wordlist `Slices` adlarının `input.*`'a dönüşmesi.
5. `RequiredPlugins` alanı: OB1 eklentisi OB2'de yok — karşılığını yerleşik blokla yaz.

## 7. Sık yapılan hata

OB1 config'ini **olduğu gibi** OB2'ye yapıştırmak ve `BLOCK:` görmek. OB1'de bloklar `BLOCK:` ile değil
`REQUEST`, `KEYCHECK` gibi çıplak anahtar sözcüklerle başlar. `node tools/lint.mjs x.loli --ob1` ile
OB1 sözdizimini de denetleyebilir, `tools/convert.mjs` ile iskeleti çevirebilirsin.
