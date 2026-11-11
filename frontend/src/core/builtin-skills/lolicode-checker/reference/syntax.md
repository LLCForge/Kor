# LoliCode sözdizimi — tam referans

Kaynak: `openbullet/OpenBullet2` master (RuriLib) `RuriLib/Helpers/LoliCode/*.cs`, `RuriLib/Models/Blocks/Custom/*.cs`,
`https://docs.openbullet.dev/docs/lolicode/`. Veri çıkarımı: `data/meta.json` (`source` alanı commit'i taşır).

## 1. Genel biçim

```
BLOCK:BlockId [LABEL:etiket] [DISABLED] [SAFE]
  ayarAdi = deger
  => VAR @cikti        (veya => CAP @cikti)
ENDBLOCK
```

- `BLOCK:` / `ENDBLOCK` zorunlu çift. Arada yalnızca ayarlar, meta satırlar ve gövde satırları bulunur.
- `LABEL:` yalnızca günlükte görünür; karar verme üzerinde etkisi yok.
- `DISABLED` bloğu atlar. `SAFE` istisnaları yakalar, `data.ERROR`'a yazar (yalnızca destekleyen bloklarda).
- Girinti kozmetiktir, anlam taşımaz.
- Çıktı: `=> VAR @ad` (değişken) veya `=> CAP @ad` (değişken + hit kaydına ekle). `@` zorunlu.

## 2. Değer tipleri (`LineParser`)

| Tip | Sözdizimi | Örnek |
|---|---|---|
| Bool | `true` / `false` (büyük/küçük duyarsız) | `autoRedirect = false` |
| Int | işaretli tam sayı, ayraçsız | `timeoutMilliseconds = 15000` |
| Float | tek nokta, ayraçsız (`0.42`) | `scaleFactor = 1.0` |
| String | çift tırnak; `\n \t \" \\ \uXXXX` kaçışları | `value = "merhaba"` |
| Enum | çıplak tanımlayıcı | `method = POST` |
| ByteArray | base64, tırnaksız | `PlW9Cg==` |
| List | `["a", "b"]` | `customCipherSuites = ["TLS_AES_128_GCM_SHA256"]` |
| Dict | `${("k","v"), ("k2","v2")}` | `customHeaders = ${("Accept", "*/*")}` |
| Interpolated | `$"..."`, `$[...]`, `${...}` + `<degisken>` | `url = $"https://x.com/<APP>"` |
| Variable | `@ad` (ayrıca `@globals.x`, `@data.SOURCE`) | `input = @data.SOURCE` |

**Önemli:** ayraç olarak virgül YOK. `1,5` float değil; `1.5` doğru. `1000` doğru, `1,000` hatalı.

## 3. Interpolasyon

- `<AD>` → değişken. `<input.USER>`, `<globals.counter>`, `<APP>`.
- Liste elemanı: `<LIST[i]>`; dikte: `<DICT(anahtar)>`; ters: `<DICT{deger}>`.
- İç içe en fazla 1 seviye: `<LIST[<COUNTER>]>`.
- `$"..."` zorunlu: düz `"..."` içindeki `<x>` genişlemez.
- Motor `<([^<>]*)>` arar; bulamazsa metni olduğu gibi bırakır (HTML `<br>` bozulmaz).

## 4. Akış ifadeleri

```
LOG "metin"                    CLOG YellowGreen "metin"
JUMP #ETIKET                   SET VAR @x "deger"        SET CAP @x "deger"
IF INTKEY 5 LessThan 3         IF STRINGKEY @src Contains "ok"
  ...
ELSE IF BOOLKEY @ok Is true
  ...
ELSE
  ...
END

REPEAT 5 ... END               FOREACH oge IN liste ... END
WHILE INTKEY 1 LessThan 2 ... END
TRY ... CATCH ... FINALLY ... END
LOCK globals ... END           ACQUIRELOCK globals ... TRY ... FINALLY RELEASELOCK globals END
MARK @x / UNMARK @x            TAKE 5 FROM "kaynak" => @liste / TAKEONE FROM "kaynak" => @tek
SET USEPROXY TRUE              SET PROXY "127.0.0.1" 9050 SOCKS5
```

- Koşul anahtar sözcükleri: `STRINGKEY`, `INTKEY`, `FLOATKEY`, `BOOLKEY`, `LISTKEY`, `DICTKEY`.
- Döngü kapanışı her zaman `END`.

## 5. KeyCheck bloğu

```
BLOCK:Keycheck
  banIfNoMatch = False
  KEYCHAIN SUCCESS OR
    STRINGKEY @data.SOURCE Contains "check\":true"
  KEYCHAIN FAIL OR
    STRINGKEY @data.SOURCE Contains "bulunamadi"
  KEYCHAIN RETRY OR
    STRINGKEY @data.SOURCE Contains "guvenlik"
ENDBLOCK
```

**Keychain durumları:** `SUCCESS`, `FAIL`, `BAN`, `RETRY`, `CUSTOM`.
**Mod:** `OR` (herhangi biri) / `AND` (hepsi).

**Operatörler (tipe göre):**

| Anahtar | Operatörler |
|---|---|
| `STRINGKEY` | EqualTo, NotEqualTo, Contains, DoesNotContain, Exists, DoesNotExist, MatchesRegex, DoesNotMatchRegex |
| `INTKEY` / `FLOATKEY` | EqualTo, NotEqualTo, LessThan, LessThanOrEqualTo, GreaterThan, GreaterThanOrEqualTo |
| `BOOLKEY` | Is, IsNot |
| `LISTKEY` | Contains, DoesNotContain, Exists, DoesNotExist |
| `DICTKEY` | HasKey, DoesNotHaveKey, HasValue, DoesNotHaveValue, Exists, DoesNotExist |

**Kritik davranış:** keychain'ler yukarıdan aşağıya değerlendirilir ve **en sondaki eşleşen kazanır**
(kaynakta "THE LAST KEY HAS ABSOLUTE PRIORITY" — özel olarak). Yani `FAIL` bloğu en üstte,
`SUCCESS` en altta olmalı; ters yazarsan SUCCESS'i FAIL ezer.

`banIfNoMatch = true` (varsayılan): hiçbir keychain tutmazsa proxy BAN edilir. Fail'de proxy banı
istemiyorsan `banIfNoMatch = False` yaz.

Var olmayan değişken: `<var>` boşsa değil, **değişken tanımlı değilse** "yok" sayılır.

## 6. HttpRequest bloğu — gövde sözdizimi

Ayarlar (`url`, `method`, `customHeaders`, `timeoutMilliseconds`, `httpLibrary`, ...) normal ayar satırlarıdır.
Gövde `TYPE:` satırıyla başlar ve sonraki satırlar konuma göre yorumlanır:

```
BLOCK:HttpRequest
  url = "https://site.com/api/login"
  method = POST
  customHeaders = ${("Content-Type", "application/json"), ("User-Agent", "<UA>")}
  TYPE:STANDARD
  $"{\"user\":\"<input.USER>\",\"pass\":\"<input.PASS>\"}"
  "application/json"
ENDBLOCK
```

| TYPE | Sonraki satırlar (sırayla) |
|---|---|
| `STANDARD` | içerik, content-type |
| `RAW` | base64 içerik, content-type |
| `BASICAUTH` | kullanıcı adı, parola |
| `MULTIPART` | boundary, sonra `CONTENT:STRING "ad" "veri" "tip"` / `CONTENT:FILE "ad" "dosya" "tip"` / `CONTENT:RAW "ad" BASE64 "tip"` |

`customCookies` bir dict'tir: `@data.COOKIES` verirsen oturum çerezleri taşınır. Tek string vermek hata olur.

HTTP kütüphaneleri: `RuriLibHttp` (varsayılan), `SystemNet` (HTTP/2-3), `CurlImpersonate`
(Chrome/Firefox TLS+JA3 parmak izi taklidi — Cloudflare/DataDome için birinci tercih).
`ignoreCertificateValidation = true` varsayılandır.

## 7. Parse bloğu

```
BLOCK:Parse
  input = @data.SOURCE
  leftDelim = "token\":\""
  rightDelim = "\""
  MODE:LR
  => VAR @token
ENDBLOCK
```

`MODE:` satırı zorunlu ve **ayar değil, meta satırdır** (`mode = LR` yazmak en yaygın hatadır).
Modlar: `LR`, `CSS`, `XPath`, `Json`, `Regex`. `RECURSIVE` satırı çoklu sonuç döner (liste).

| MODE | Zorunlu ayarlar |
|---|---|
| LR | leftDelim, rightDelim |
| CSS | cssSelector (+ attributeName) |
| XPath | xPath (+ attributeName) |
| Json | jToken |
| Regex | pattern (+ outputFormat: `[0]` tam, `[1..]` gruplar) |

Ortak: `prefix`, `suffix`, `urlEncodeOutput`, `caseSensitive`.

**OB1 tuzağı:** OB1 1.2.2'de XPath modu `NotImplementedException` fırlatır; OB2'de çalışır.

## 8. Script blokları

```
BLOCK:Script
  interpreter = Jint
  input = STRING
  script = $"script.Calculate(<A>, <B>)"
  => VAR @sonuc
ENDBLOCK
```

`interpreter`: `Jint` (JS), `NodeJS`, `IronPython`, `Python`. `input = STRING` değişken enjeksiyonu yapar.
`.RuriLib`/`.NET` köprüsü ile C#/JS tarafına geçilir. Uzun hesap işlerini buraya taşı (±%10 kuralı).

## 9. Yorumlar

OB2 LoliCode: `//` satır başı yorum. OB1 LoliScript: `##` yorum, `!` devre dışı satır.
