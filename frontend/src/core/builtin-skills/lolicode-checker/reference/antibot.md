# Anti-bot ve atlatma rehberi — hangi katman, hangi araç

Kaynak: OpenBullet2 blok kataloğu (`data/blocks.json`, Captchas + Browser + HttpRequest), doküman
ve saha deneyimi. Hedef: hedef siteye "gerçek tarayıcı" gibi görünmek, çözülemeyeni çözmek.

## 1. Karar ağacı (önce bunu uygula)

1. **Sadece rate-limit / "güvenlik nedeniyle giriş yapılamadı"** → proxy rotasyonu + `Delay`
   (+ gerekirse `LOCK globals` ile global sayaç) + `httpLibrary = RuriLibHttp`. Tarayıcı açma.
2. **TLS parmak izi (JA3) kontrolü** → `httpLibrary = CurlImpersonate` +
   `curlImpersonateBrowserProfile = Chrome142` + `curlUseBrowserHeaders = true`.
   Bu tek değişiklik, çoğu Cloudflare "bot" reddini kaldırır ve Puppeteer'dan 50× ucuzdur.
3. **JS ile üretilen token (imza)** → iki yol:
   - Token JS'i saf/okunabilirse: `Script` bloğu (`Jint`) ile kendin hesapla — en hızlı ve proxy'siz.
   - JS obfuscated/bağımlıysa: `BrowserNavigateTo` + `BrowserExecuteJs` ile **tek tarayıcı** aç,
     token'ı al, kalan istekleri HTTP ile yap, `BrowserCloseTab`.
4. **JS challenge sayfası (Cloudflare IUAM, DataDome, AWS WAF)** → `SolveCloudflareChallengePage`
   / `SolveDataDomeCaptcha` / `SolveAmazonWafCaptcha` blokları + çözüm servisi anahtarı.
5. **Görsel/checkbox captcha** → `SolveImageCaptcha`, `SolveRecaptchaV2/V3`, `SolveHCaptcha`,
   `SolveCloudflareTurnstileCaptcha`, `SolveFunCaptcha`, `SolveGeeTestCaptcha`, `SolveMtCaptcha`.
   Turnstile için `action`/`cData` alanlarını sayfadan oku.
6. **SMS/2FA/OTP** → `status = CUSTOM` + `LASTID` ile durumu kaydet; yeni bot `Sms2Fa` datasıyla
   aynı çerez kavanozunu kullanarak `pinx`/`smsCodex` ile tamamlar. IMAP bloğu ile e-posta OTP okunur.

## 2. Tarayıcı motorları

| Motor | Ne zaman | Bloklar |
|---|---|---|
| HTTP | varsayılan, en hızlı | `HttpRequest` |
| curl-impersonate | TLS parmak izi isteyen CF/DataDome | `HttpRequest` + `httpLibrary = CurlImpersonate` |
| Puppeteer/Playwright | JS token, SPA, element etkileşimi | `Browser*` (`BrowserOpen`, `BrowserNavigateTo`, `BrowserExecuteJs`, `BrowserClick`, `BrowserWaitForElement`) |
| Selenium | legacy, WebDriver tabanlı | `Selenium*` |

Puppeteer/Playwright blokları `BrowserOpen`'da `extraCmdLineArgs` alır (`--disable-blink-features=AutomationControlled` gibi).
`BrowserAutomationEngine = Puppeteer | Playwright` ayarı motoru seçer. `BrowserOpen` yerine `BrowserNewTab` kullan.

**Puppeteer maliyeti:** her tarayıcı ~200-400 MB RAM. 50 bot × tarayıcı = makine ölür. Kural: tarayıcı
yalnızca zorunluysa, mümkünse bot başına 1 tarayıcı ve iş bitince kapat.

## 3. Kaynak metninde aranacak gerçek anahtarlar (uydurma değil)

Bot tespitinde işe yarayan somut göstergeler:

- HTTP 429 / 503 + `Retry-After`
- Cloudflare: `__cf_bm`, `cf_clearance`, `cf-mitigated: challenge`, `Just a moment...`
- DataDome: `datadome` çerezi, `x-datadome` başlığı
- PerimeterX: `_px`, `pxhd`, `x-px-*`
- Akamai: `_abck`, `bm_sz`, `ak_bmsc`
- AWS WAF: `aws-waf-token`, `challenge.js`, `captcha.js`
- Imperva/Incapsula: `visid_incap`, `incap_ses`, `nlbi`
- hCaptcha: `h-captcha`, `hcaptcha.com`, `sitekey`
- reCAPTCHA: `g-recaptcha`, `recaptcha/api.js`, `data-sitekey`, `grecaptcha.enterprise`
- Turnstile: `cf-turnstile`, `challenges.cloudflare.com/turnstile`

KeyCheck'e bunları yazarsan Ban/Retry ayrımı doğru olur — asıl kazanç buradadır, çözüm servisinde değil.

## 4. Keşif yöntemi (config yazmadan önce 10 dakika)

1. Hedefi curl/pw ile aç, **DevTools Network** kaydını al, giriş akışını tek tek yürü.
2. Şu üçünü bul: (a) gerçek giriş endpoint'i, (b) **JS tarafından üretilen** alanlar,
   (c) oturum çerezlerinin adları ve ömrü.
3. Token JS'i bul (`AuthSec.getSecToken`, `encrypt(`, `sign(`, `_token`, `nonce` ara).
   Okunabilirse `Script` bloğuna çevir; değilse tarayıcıya bırak ve `BrowserExecuteJs` ile al.
4. `dinamo_session` gibi oturum çerezini bul → sonraki isteklerde `customCookies = @data.COOKIES`.
5. Google'da config'i ara: `"<domain>" "<endpoint-adı>" openbullet config` — hazır config varsa
   gereksiz keşiften kurtulursun. Ama **kodu oku**, körü körüne kullanma.

## 5. Operasyonel parametreler (aşırıya kaçma)

- Bot sayısı: hedefin toleransı kadar. 429 görmeden 1 bot, 429 görünce 0.5'e in.
  `Note: her bot = 1 eşzamanlı istek` → 20 bot, hedefin saniyede 20 isteğe toleransı demek.
- `Delay` 300-1500 ms: gerçek kullanıcı davranışı. `RandomInteger` ile 200-900 gibi rastgele ver.
- Yalnızca zorunluysa tarayıcı; yalnızca zorunluysa captcha servisi (para).
- Çözüm servisi anahtarını `input.` üzerinden ver, koda gömme; bakiye kontrolünü `BypassBalanceCheck`
  yerine başta bir HTTP isteği ile yap (açık bakiye kontrolü daha iyi bir hata mesajı verir).

## 6. Sık patlayan noktalar

| Belirti | Gerçek sebep | Çözüm |
|---|---|---|
| "Girmiş olduğunuz bilgilere ait kullanıcı bulunamadı" ama şifre doğru | alan adı/parametre adı hatalı, token eksik | endpoint'i DevTools'tan birebir kopyala |
| Her denemede 302 → /tr/ (aynı sayfa) | çerez taşınmıyor | `customCookies = @data.COOKIES` |
| `cf_clearance` var ama yine challenge | proxy değişti | sticky proxy |
| Turnstile/mtcaptcha geçiyor ama reddediliyor | `action`/`cData` sayfadan okunmadı | alanları `Parse` ile al, blok parametresine ver |
| Başarılı giriş `FAIL` sayılıyor | KeyCheck'te string yanlış kaçışlanmış | JSON yanıtı için `\"` kullan, `Contains` kontrolünü gerçek anahtara bağla |
| `SET PROXY` sonrası istek proxy'siz | `data.UseProxy = false` | `SET USEPROXY TRUE` |

## 7. Kaçış / yönlendirme

Tıkanırsan sırayla: HTTP → curl-impersonate → Puppeteer → çözüm servisi → başka bir giriş yolu
(mobil API, `wp-json`, `graphql`, `/api/v2`) . Çoğu zaman bir alt yolda token kontrolü yoktur.
`Script` bloğu ile obfuscated JS'i kısmen yeniden yazmak, tam tarayıcıdan her zaman daha ucuzdur.
