//! Payment integration templates (PHP) for the providers whose integration contract is publicly
//! documented: iyzico Checkout Form (official `iyzico/iyzipay-php` SDK), PayTR iFrame API (token
//! and callback hashes), Param POS (TurkPos SOAP service, <https://dev.param.com.tr>) 3D model
//! and Sipay (`/ccpayment/api`, <https://apidocs.sipay.com.tr>) 3D payment. Keys are read from
//! environment variables, never written into code. Every template confirms the result with the
//! provider server-to-server before an order is marked paid.

/// (file name, content) pairs for `provider`: `iyzico`, `paytr`, `param` or `sipay`.
pub fn payment(provider: &str) -> Option<Vec<(String, String)>> {
    match provider {
        "iyzico" => Some(vec![
            (
                "iyzico/composer.json".into(),
                "{\n  \"require\": {\n    \"iyzico/iyzipay-php\": \"^2.0\"\n  }\n}\n".into(),
            ),
            ("iyzico/checkout.php".into(), IYZICO.into()),
            ("iyzico/callback.php".into(), IYZICO_CALLBACK.into()),
        ]),
        "paytr" => Some(vec![
            ("paytr/token.php".into(), PAYTR.into()),
            ("paytr/callback.php".into(), PAYTR_CALLBACK.into()),
        ]),
        "param" => Some(vec![
            ("param/param.php".into(), PARAM.into()),
            ("param/callback.php".into(), PARAM_CALLBACK.into()),
        ]),
        "sipay" => Some(vec![
            ("sipay/sipay.php".into(), SIPAY.into()),
            ("sipay/callback.php".into(), SIPAY_CALLBACK.into()),
        ]),
        _ => None,
    }
}

pub const PROVIDERS: [&str; 4] = ["iyzico", "paytr", "param", "sipay"];

const PARAM: &str = r#"<?php
// Param POS (TurkPos) 3D model over SOAP: SHA2B64 → TP_WMD_UCD → bank 3D page → callback.php →
// TP_WMD_Pay. Reference: https://dev.param.com.tr (Ödeme / TP_WMD_UCD, TP_WMD_Pay, SHA2B64).
// Keys: PARAM_CLIENT_CODE, PARAM_CLIENT_USERNAME, PARAM_CLIENT_PASSWORD, PARAM_GUID.
// PARAM_WSDL: https://testposws.param.com.tr/turkpos.ws/service_turkpos_prod.asmx?wsdl while
// testing (Param publishes test credentials there), https://posws.param.com.tr/turkpos.ws/service_turkpos_prod.asmx?wsdl live.

function kor_param_client(): SoapClient {
    $wsdl = getenv('PARAM_WSDL') ?: 'https://testposws.param.com.tr/turkpos.ws/service_turkpos_prod.asmx?wsdl';
    return new SoapClient($wsdl, ['exceptions' => true, 'connection_timeout' => 20]);
}

function kor_param_auth(): array {
    return [
        'CLIENT_CODE' => getenv('PARAM_CLIENT_CODE'),
        'CLIENT_USERNAME' => getenv('PARAM_CLIENT_USERNAME'),
        'CLIENT_PASSWORD' => getenv('PARAM_CLIENT_PASSWORD'),
    ];
}

/** Param's own SHA2B64 method, so the hash is computed exactly as Param checks it. */
function kor_param_hash(SoapClient $c, string $data): string {
    return $c->SHA2B64(['Data' => $data])->SHA2B64Result;
}

/**
 * Starts a 3D payment and returns the bank's 3D HTML to print (the browser posts back to $okUrl/$failUrl).
 * Amounts use a comma decimal separator, e.g. "100,00".
 */
function kor_param_start(array $order, array $card, string $okUrl, string $failUrl): string {
    $c = kor_param_client();
    $guid = getenv('PARAM_GUID');
    $installment = (string) ($order['installment'] ?? '1');
    $hashStr = getenv('PARAM_CLIENT_CODE') . $guid . $installment . $order['amount'] . $order['total'] . $order['id'];
    $r = $c->TP_WMD_UCD([
        'G' => kor_param_auth(),
        'GUID' => $guid,
        'KK_Sahibi' => $card['holder'],
        'KK_No' => $card['number'],
        'KK_SK_Ay' => $card['month'],
        'KK_SK_Yil' => $card['year'],
        'KK_CVC' => $card['cvc'],
        'KK_Sahibi_GSM' => $order['phone'] ?? '',
        'Hata_URL' => $failUrl,
        'Basarili_URL' => $okUrl,
        'Siparis_ID' => $order['id'],
        'Siparis_Aciklama' => $order['description'] ?? '',
        'Taksit' => $installment,
        'Islem_Tutar' => $order['amount'],
        'Toplam_Tutar' => $order['total'],
        'Islem_Hash' => kor_param_hash($c, $hashStr),
        'Islem_Guvenlik_Tip' => '3D',
        'Islem_ID' => '',
        'IPAdr' => $_SERVER['REMOTE_ADDR'] ?? '',
        'Ref_URL' => '',
        'Data1' => '', 'Data2' => '', 'Data3' => '', 'Data4' => '', 'Data5' => '',
    ])->TP_WMD_UCDResult;
    if ((int) $r->Sonuc <= 0) {
        throw new RuntimeException('Param: ' . $r->Sonuc_Str);
    }
    return $r->UCD_HTML;
}
"#;

const PARAM_CALLBACK: &str = r"<?php
// The bank posts the 3D result here (Basarili_URL). Only a 3D-verified card is charged, with TP_WMD_Pay.
require __DIR__ . '/param.php';
$mdStatus = (string) ($_POST['mdStatus'] ?? '');
if (!in_array($mdStatus, ['1', '2', '3', '4'], true)) {
    http_response_code(400);
    exit('3D doğrulaması başarısız');
}
$c = kor_param_client();
$r = $c->TP_WMD_Pay([
    'G' => kor_param_auth(),
    'GUID' => getenv('PARAM_GUID'),
    'UCD_MD' => (string) ($_POST['md'] ?? ''),
    'Islem_GUID' => (string) ($_POST['islemGUID'] ?? ''),
    'Siparis_ID' => (string) ($_POST['orderId'] ?? ''),
])->TP_WMD_PayResult;
if ((int) $r->Sonuc > 0 && (int) $r->Dekont_ID > 0) {
    // Mark order $_POST['orderId'] as paid once (check the amount against your own order first).
} else {
    // Payment failed: $r->Sonuc_Ack
}
";

const SIPAY: &str = r#"<?php
// Sipay 3D payment (paySmart3D). Reference: https://apidocs.sipay.com.tr (Token Generation,
// 3D Secure Payment, Check Status, Hash Creation). Keys: SIPAY_APP_ID, SIPAY_APP_SECRET,
// SIPAY_MERCHANT_KEY. SIPAY_BASE_URL is https://provisioning.sipay.com.tr while testing and
// https://app.sipay.com.tr in production.

function kor_sipay_base(): string {
    return rtrim(getenv('SIPAY_BASE_URL') ?: 'https://provisioning.sipay.com.tr', '/');
}

function kor_sipay_post(string $path, array $body, ?string $token = null): array {
    $h = ['Content-Type: application/json', 'Accept: application/json'];
    if ($token !== null) {
        $h[] = 'Authorization: Bearer ' . $token;
    }
    $ch = curl_init(kor_sipay_base() . $path);
    curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_POST => true, CURLOPT_HTTPHEADER => $h,
        CURLOPT_POSTFIELDS => json_encode($body), CURLOPT_TIMEOUT => 30]);
    $res = curl_exec($ch);
    if ($res === false) {
        throw new RuntimeException('Sipay bağlantı hatası: ' . curl_error($ch));
    }
    return json_decode($res, true) ?: [];
}

/** Bearer token, valid for two hours (cache it). */
function kor_sipay_token(): string {
    $r = kor_sipay_post('/ccpayment/api/token', [
        'app_id' => getenv('SIPAY_APP_ID'),
        'app_secret' => getenv('SIPAY_APP_SECRET'),
    ]);
    if (($r['status_code'] ?? -1) !== 100 || empty($r['data']['token'])) {
        throw new RuntimeException('Sipay token: ' . ($r['status_description'] ?? 'bilinmeyen hata'));
    }
    return $r['data']['token'];
}

/**
 * Sipay's hash_key: the fields joined with '|', AES-256-CBC with key sha256(sha1(app_secret) . salt),
 * sent as iv:salt:ciphertext with '/' replaced by '__' (Hash Creation page). The field list differs
 * per endpoint; compare it with the endpoint's sample code before going live.
 */
function kor_sipay_hash(array $fields): string {
    $data = implode('|', $fields);
    $iv = substr(sha1((string) random_int(0, PHP_INT_MAX)), 0, 16);
    $password = sha1(getenv('SIPAY_APP_SECRET'));
    $salt = substr(sha1((string) random_int(0, PHP_INT_MAX)), 0, 4);
    $key = hash('sha256', $password . $salt);
    $encrypted = openssl_encrypt($data, 'aes-256-cbc', $key, 0, $iv);
    return str_replace('/', '__', "$iv:$salt:$encrypted");
}

/** Returns the HTML of the 3D step to print. Amounts as "100.00", currency "TRY". */
function kor_sipay_pay3d(array $order, array $card, array $customer, string $okUrl, string $failUrl): string {
    $merchantKey = getenv('SIPAY_MERCHANT_KEY');
    $installments = (string) ($order['installments'] ?? '1');
    $body = [
        'cc_holder_name' => $card['holder'], 'cc_no' => $card['number'],
        'expiry_month' => $card['month'], 'expiry_year' => $card['year'], 'cvv' => $card['cvv'],
        'currency_code' => 'TRY', 'installments_number' => $installments,
        'invoice_id' => $order['id'], 'invoice_description' => $order['description'] ?? '',
        'name' => $customer['name'], 'surname' => $customer['surname'],
        'total' => $order['total'], 'merchant_key' => $merchantKey,
        'items' => json_encode($order['items']),
        'cancel_url' => $failUrl, 'return_url' => $okUrl,
        'hash_key' => kor_sipay_hash([$order['total'], $installments, 'TRY', $merchantKey, $order['id']]),
        'ip' => $_SERVER['REMOTE_ADDR'] ?? '', 'response_method' => 'POST',
        'bill_email' => $customer['email'], 'bill_phone' => $customer['phone'],
        'bill_address1' => $customer['address'], 'app_lang' => 'tr',
    ];
    $ch = curl_init(kor_sipay_base() . '/ccpayment/api/paySmart3D');
    curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_POST => true, CURLOPT_TIMEOUT => 30,
        CURLOPT_HTTPHEADER => ['Content-Type: application/json', 'Authorization: Bearer ' . kor_sipay_token()],
        CURLOPT_POSTFIELDS => json_encode($body)]);
    $html = curl_exec($ch);
    if ($html === false) {
        throw new RuntimeException('Sipay bağlantı hatası: ' . curl_error($ch));
    }
    return $html;
}

/** Server-to-server check of an invoice (Check Status); trust this, not the browser redirect. */
function kor_sipay_status(string $invoiceId): array {
    $merchantKey = getenv('SIPAY_MERCHANT_KEY');
    return kor_sipay_post('/ccpayment/api/checkstatus', [
        'merchant_key' => $merchantKey,
        'invoice_id' => $invoiceId,
        'include_pending_status' => 'true',
        'hash_key' => kor_sipay_hash([$invoiceId, $merchantKey]),
    ], kor_sipay_token());
}
"#;

const SIPAY_CALLBACK: &str = r"<?php
// return_url / cancel_url target. The browser's fields are not trusted: the invoice is checked with Sipay.
require __DIR__ . '/sipay.php';
$invoiceId = (string) ($_POST['invoice_id'] ?? '');
if ($invoiceId === '') {
    http_response_code(400);
    exit;
}
$s = kor_sipay_status($invoiceId);
if (($s['status_code'] ?? -1) === 100 && ($s['transaction_status'] ?? '') === 'Completed') {
    // Mark order $invoiceId as paid once (check $s['product_price'] against your own order first).
} else {
    // Payment failed or pending: $s['status_description']
}
";

const IYZICO: &str = r"<?php
// iyzico Checkout Form (iyzipay-php). Keys: IYZICO_API_KEY, IYZICO_SECRET_KEY; IYZICO_BASE_URL is
// https://sandbox-api.iyzipay.com while testing and https://api.iyzipay.com in production.
require_once __DIR__ . '/vendor/autoload.php';

function kor_iyzico_options(): \Iyzipay\Options {
    $o = new \Iyzipay\Options();
    $o->setApiKey(getenv('IYZICO_API_KEY'));
    $o->setSecretKey(getenv('IYZICO_SECRET_KEY'));
    $o->setBaseUrl(getenv('IYZICO_BASE_URL') ?: 'https://sandbox-api.iyzipay.com');
    return $o;
}

/** @param array{id:string,total:string,items:array<int,array{id:string,name:string,category:string,price:string}>} $order */
function kor_iyzico_checkout(array $order, array $customer, string $callbackUrl): string {
    $r = new \Iyzipay\Request\CreateCheckoutFormInitializeRequest();
    $r->setLocale(\Iyzipay\Model\Locale::TR);
    $r->setConversationId($order['id']);
    $r->setPrice($order['total']);
    $r->setPaidPrice($order['total']);
    $r->setCurrency(\Iyzipay\Model\Currency::TL);
    $r->setBasketId($order['id']);
    $r->setPaymentGroup(\Iyzipay\Model\PaymentGroup::PRODUCT);
    $r->setCallbackUrl($callbackUrl);

    $b = new \Iyzipay\Model\Buyer();
    $b->setId($customer['id']);
    $b->setName($customer['name']);
    $b->setSurname($customer['surname']);
    $b->setEmail($customer['email']);
    $b->setIdentityNumber($customer['identity_number']);
    $b->setRegistrationAddress($customer['address']);
    $b->setIp($_SERVER['REMOTE_ADDR'] ?? '');
    $b->setCity($customer['city']);
    $b->setCountry('Turkey');
    $r->setBuyer($b);

    $a = new \Iyzipay\Model\Address();
    $a->setContactName($customer['name'] . ' ' . $customer['surname']);
    $a->setCity($customer['city']);
    $a->setCountry('Turkey');
    $a->setAddress($customer['address']);
    $r->setShippingAddress($a);
    $r->setBillingAddress($a);

    $items = [];
    foreach ($order['items'] as $it) {
        $i = new \Iyzipay\Model\BasketItem();
        $i->setId($it['id']);
        $i->setName($it['name']);
        $i->setCategory1($it['category']);
        $i->setItemType(\Iyzipay\Model\BasketItemType::PHYSICAL);
        $i->setPrice($it['price']);
        $items[] = $i;
    }
    $r->setBasketItems($items);

    $form = \Iyzipay\Model\CheckoutFormInitialize::create($r, kor_iyzico_options());
    if ($form->getStatus() !== 'success') {
        error_log('iyzico: ' . $form->getErrorMessage());
        throw new RuntimeException('Ödeme formu açılamadı');
    }
    return $form->getCheckoutFormContent(); // embed in the page; it renders the form
}
";

const IYZICO_CALLBACK: &str = r"<?php
// iyzico posts a token here; the payment result is read from iyzico with that token (never trusted from the browser).
require_once __DIR__ . '/checkout.php';

$token = isset($_POST['token']) ? preg_replace('/[^A-Za-z0-9-]/', '', (string) $_POST['token']) : '';
if ($token === '') {
    http_response_code(400);
    exit;
}
$r = new \Iyzipay\Request\RetrieveCheckoutFormRequest();
$r->setLocale(\Iyzipay\Model\Locale::TR);
$r->setToken($token);
$result = \Iyzipay\Model\CheckoutForm::retrieve($r, kor_iyzico_options());
if ($result->getPaymentStatus() === 'SUCCESS') {
    // Mark order $result->getBasketId() as paid (check the amount against your own order first).
} else {
    // Payment failed: $result->getErrorMessage()
}
";

const PAYTR: &str = r#"<?php
// PayTR iFrame API: get a token, then show https://www.paytr.com/odeme/guvenli/<token> in an iframe.
// Keys: PAYTR_MERCHANT_ID, PAYTR_MERCHANT_KEY, PAYTR_MERCHANT_SALT. PAYTR_TEST_MODE=1 while testing.

/** @param array<int,array{0:string,1:string,2:int}> $basket [name, unit price "18.00", quantity] */
function kor_paytr_token(string $orderId, string $email, int $amountKurus, array $basket, array $customer, string $okUrl, string $failUrl): string {
    $merchantId = getenv('PAYTR_MERCHANT_ID');
    $key = getenv('PAYTR_MERCHANT_KEY');
    $salt = getenv('PAYTR_MERCHANT_SALT');
    $testMode = getenv('PAYTR_TEST_MODE') ?: '0';
    $ip = $_SERVER['REMOTE_ADDR'] ?? '';
    $userBasket = base64_encode(json_encode($basket));
    $noInstallment = '0';
    $maxInstallment = '0';
    $currency = 'TL';
    $hashStr = $merchantId . $ip . $orderId . $email . $amountKurus . $userBasket . $noInstallment . $maxInstallment . $currency . $testMode;
    $token = base64_encode(hash_hmac('sha256', $hashStr . $salt, $key, true));
    $post = [
        'merchant_id' => $merchantId, 'user_ip' => $ip, 'merchant_oid' => $orderId, 'email' => $email,
        'payment_amount' => $amountKurus, 'paytr_token' => $token, 'user_basket' => $userBasket,
        'debug_on' => $testMode, 'no_installment' => $noInstallment, 'max_installment' => $maxInstallment,
        'user_name' => $customer['name'], 'user_address' => $customer['address'], 'user_phone' => $customer['phone'],
        'merchant_ok_url' => $okUrl, 'merchant_fail_url' => $failUrl, 'timeout_limit' => '30',
        'currency' => $currency, 'test_mode' => $testMode,
    ];
    $ch = curl_init('https://www.paytr.com/odeme/api/get-token');
    curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_POST => true, CURLOPT_POSTFIELDS => $post, CURLOPT_TIMEOUT => 20]);
    $res = curl_exec($ch);
    if ($res === false) {
        throw new RuntimeException('PayTR bağlantı hatası: ' . curl_error($ch));
    }
    $json = json_decode($res, true);
    if (($json['status'] ?? '') !== 'success') {
        throw new RuntimeException('PayTR: ' . ($json['reason'] ?? 'bilinmeyen hata'));
    }
    return $json['token'];
}
"#;

const PAYTR_CALLBACK: &str = r"<?php
// PayTR notifies the result here (Bildirim URL). The hash is checked before anything is trusted.
$key = getenv('PAYTR_MERCHANT_KEY');
$salt = getenv('PAYTR_MERCHANT_SALT');
$oid = (string) ($_POST['merchant_oid'] ?? '');
$status = (string) ($_POST['status'] ?? '');
$total = (string) ($_POST['total_amount'] ?? '');
$hash = base64_encode(hash_hmac('sha256', $oid . $salt . $status . $total, $key, true));
if (!hash_equals($hash, (string) ($_POST['hash'] ?? ''))) {
    http_response_code(400);
    exit('PAYTR notification failed: bad hash');
}
if ($status === 'success') {
    // Mark order $oid as paid once (this URL may be called more than once).
} else {
    // Payment failed: $_POST['failed_reason_msg']
}
echo 'OK';
";

#[cfg(test)]
mod tests {
    #[test]
    fn templates_never_embed_keys() {
        for p in super::PROVIDERS {
            let files = super::payment(p).unwrap();
            assert!(!files.is_empty());
            for (_, text) in files {
                assert!(text.contains("getenv(") || text.contains("require"), "{p}");
                assert!(!text.contains("sk_live") && !text.contains("YOUR_"));
            }
        }
        assert!(super::payment("x").is_none());
        assert!(super::PAYTR_CALLBACK.contains("hash_equals"));
        // Param charges only after 3D verification; Sipay confirms server-to-server.
        assert!(
            super::PARAM_CALLBACK.contains("TP_WMD_Pay")
                && super::PARAM_CALLBACK.contains("mdStatus")
        );
        assert!(super::SIPAY_CALLBACK.contains("kor_sipay_status"));
    }
}
