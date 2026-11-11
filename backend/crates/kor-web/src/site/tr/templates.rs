//! Payment integration templates (PHP) for the providers whose public integration contract is
//! documented: iyzico Checkout Form (official `iyzico/iyzipay-php` SDK) and PayTR iFrame API
//! (token and callback hashes). Keys are read from environment variables, never written into code.
//! Param and Sipay are detected on sites (see the parent module); their integrations need the
//! provider's merchant documentation and are not templated.

/// (file name, content) pairs for `provider`: `iyzico` or `paytr`.
pub fn payment(provider: &str) -> Option<Vec<(String, String)>> {
    match provider {
        "iyzico" => Some(vec![
            ("iyzico/composer.json".into(), "{\n  \"require\": {\n    \"iyzico/iyzipay-php\": \"^2.0\"\n  }\n}\n".into()),
            ("iyzico/checkout.php".into(), IYZICO.into()),
            ("iyzico/callback.php".into(), IYZICO_CALLBACK.into()),
        ]),
        "paytr" => Some(vec![
            ("paytr/token.php".into(), PAYTR.into()),
            ("paytr/callback.php".into(), PAYTR_CALLBACK.into()),
        ]),
        _ => None,
    }
}

pub const PROVIDERS: [&str; 2] = ["iyzico", "paytr"];

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
    }
}
