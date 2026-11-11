//! E-commerce/CMS recognition (§29.3 adapters): WordPress/WooCommerce, OpenCart, PrestaShop,
//! Magento, Shopify, and the Turkish SaaS shops Ticimax, İdeasoft and T-Soft — from a project
//! folder (file layout and version constants) or a live page (HTML fingerprints). Module skeletons
//! for the self-hosted platforms (OpenCart 4 extension, PrestaShop module). SaaS shops accept no
//! server modules; for them Kor works through their admin panels and APIs.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Detected {
    /// `wordpress`, `woocommerce`, `opencart`, `prestashop`, `magento`, `shopify`, `ticimax`, `ideasoft`, `tsoft`, `joomla`, `drupal`.
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// File or marker it was recognised by.
    pub evidence: String,
    /// Kor can add server-side modules (self-hosted) or not (SaaS).
    pub modules: bool,
}

fn grab(text: &str, re: &str) -> Option<String> {
    regex::Regex::new(re).ok()?.captures(text).map(|c| c[1].to_owned())
}

/// Platforms in a local project folder.
pub fn detect_dir(root: &Path) -> Vec<Detected> {
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).unwrap_or_default();
    let mut out = Vec::new();
    if root.join("wp-includes/version.php").is_file() || root.join("wp-config.php").is_file() {
        out.push(Detected {
            platform: "wordpress".into(),
            version: grab(&read("wp-includes/version.php"), r"\$wp_version\s*=\s*'([^']+)'"),
            evidence: "wp-includes/version.php".into(),
            modules: true,
        });
        if root.join("wp-content/plugins/woocommerce/woocommerce.php").is_file() {
            out.push(Detected {
                platform: "woocommerce".into(),
                version: grab(&read("wp-content/plugins/woocommerce/woocommerce.php"), r"Version:\s*([0-9.]+)"),
                evidence: "wp-content/plugins/woocommerce".into(),
                modules: true,
            });
        }
    }
    if root.join("system/startup.php").is_file() && root.join("catalog").is_dir() {
        out.push(Detected {
            platform: "opencart".into(),
            version: grab(&read("admin/index.php"), r"define\('VERSION',\s*'([^']+)'\)").or_else(|| grab(&read("index.php"), r"define\('VERSION',\s*'([^']+)'\)")),
            evidence: "system/startup.php".into(),
            modules: true,
        });
    }
    if root.join("config/defines.inc.php").is_file() || root.join("classes/PrestaShopAutoload.php").is_file() {
        out.push(Detected {
            platform: "prestashop".into(),
            version: grab(&read("app/AppKernel.php"), r"const VERSION = '([^']+)'").or_else(|| grab(&read("config/settings.inc.php"), r"_PS_VERSION_',\s*'([^']+)'")),
            evidence: "config/defines.inc.php".into(),
            modules: true,
        });
    }
    if root.join("bin/magento").is_file() {
        out.push(Detected { platform: "magento".into(), version: None, evidence: "bin/magento".into(), modules: true });
    }
    out
}

/// Platforms behind a live page's HTML.
pub fn detect_html(html: &str) -> Vec<Detected> {
    let l = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut add = |p: &str, version: Option<String>, ev: &str, modules: bool| {
        out.push(Detected { platform: p.into(), version, evidence: ev.into(), modules });
    };
    if l.contains("/wp-content/") || l.contains("content=\"wordpress") {
        add("wordpress", super::wp::generator_version(html), "/wp-content/", true);
    }
    if l.contains("woocommerce") {
        add("woocommerce", None, "woocommerce", true);
    }
    if l.contains("catalog/view/theme") || l.contains("route=common/home") || l.contains("index.php?route=product") {
        add("opencart", None, "catalog/view/theme", true);
    }
    if l.contains("var prestashop") || l.contains("content=\"prestashop") || l.contains("/modules/ps_") {
        add("prestashop", None, "prestashop", true);
    }
    if l.contains("mage/cookies") || l.contains("magento") {
        add("magento", None, "magento", true);
    }
    if l.contains("cdn.shopify.com") {
        add("shopify", None, "cdn.shopify.com", false);
    }
    if l.contains("ticimax") {
        add("ticimax", None, "ticimax", false);
    }
    if l.contains("ideasoft") || l.contains("myideasoft") {
        add("ideasoft", None, "ideasoft", false);
    }
    if l.contains("t-soft") || l.contains("tsoft") {
        add("tsoft", None, "t-soft", false);
    }
    if l.contains("content=\"joomla") {
        add("joomla", None, "generator Joomla", true);
    }
    if l.contains("content=\"drupal") {
        add("drupal", None, "generator Drupal", true);
    }
    out
}

/// `kargo-takip` → `KargoTakip`.
fn pascal(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

/// OpenCart 4 extension (zip the folder as `<code>.ocmod.zip` to install).
pub fn opencart_module(code: &str, name: &str) -> Vec<(String, String)> {
    let class = pascal(code);
    let c = code.replace('-', "_");
    vec![
        (
            format!("{code}/install.json"),
            format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"author\": \"\",\n  \"link\": \"\"\n}}\n"),
        ),
        (
            format!("{code}/admin/controller/module/{c}.php"),
            format!(
                "<?php\nnamespace Opencart\\Admin\\Controller\\Extension\\{class}\\Module;\n\nclass {class} extends \\Opencart\\System\\Engine\\Controller {{\n\
\tpublic function index(): void {{\n\t\t$this->load->language('extension/{c}/module/{c}');\n\t\t$this->document->setTitle($this->language->get('heading_title'));\n\
\t\t$data['save'] = $this->url->link('extension/{c}/module/{c}.save', 'user_token=' . $this->session->data['user_token']);\n\
\t\t$data['module_{c}_status'] = $this->config->get('module_{c}_status');\n\t\t$data['header'] = $this->load->controller('common/header');\n\
\t\t$data['column_left'] = $this->load->controller('common/column_left');\n\t\t$data['footer'] = $this->load->controller('common/footer');\n\
\t\t$this->response->setOutput($this->load->view('extension/{c}/module/{c}', $data));\n\t}}\n\n\
\tpublic function save(): void {{\n\t\t$this->load->language('extension/{c}/module/{c}');\n\t\t$json = [];\n\
\t\tif (!$this->user->hasPermission('modify', 'extension/{c}/module/{c}')) {{\n\t\t\t$json['error']['warning'] = $this->language->get('error_permission');\n\t\t}}\n\
\t\tif (!$json) {{\n\t\t\t$this->load->model('setting/setting');\n\t\t\t$status = isset($this->request->post['module_{c}_status']) ? (int)$this->request->post['module_{c}_status'] : 0;\n\
\t\t\t$this->model_setting_setting->editSetting('module_{c}', ['module_{c}_status' => $status]);\n\t\t\t$json['success'] = $this->language->get('text_success');\n\t\t}}\n\
\t\t$this->response->addHeader('Content-Type: application/json');\n\t\t$this->response->setOutput(json_encode($json));\n\t}}\n}}\n"
            ),
        ),
        (
            format!("{code}/admin/language/en-gb/module/{c}.php"),
            format!("<?php\n$_['heading_title'] = '{name}';\n$_['text_success'] = 'Success: settings saved.';\n$_['entry_status'] = 'Status';\n$_['error_permission'] = 'Warning: you do not have permission to modify this module.';\n"),
        ),
        (
            format!("{code}/admin/language/tr-tr/module/{c}.php"),
            format!("<?php\n$_['heading_title'] = '{name}';\n$_['text_success'] = 'Ayarlar kaydedildi.';\n$_['entry_status'] = 'Durum';\n$_['error_permission'] = 'Uyarı: bu modülü değiştirme yetkiniz yok.';\n"),
        ),
        (
            format!("{code}/admin/view/template/module/{c}.twig"),
            format!(
                "{{{{ header }}}}{{{{ column_left }}}}\n<div id=\"content\" class=\"container-fluid\">\n  <h1>{{{{ heading_title }}}}</h1>\n\
  <form id=\"form-module\" action=\"{{{{ save }}}}\" method=\"post\" data-oc-toggle=\"ajax\">\n\
    <label for=\"input-status\">{{{{ entry_status }}}}</label>\n\
    <input type=\"checkbox\" name=\"module_{c}_status\" value=\"1\" id=\"input-status\"{{% if module_{c}_status %}} checked{{% endif %}}/>\n\
    <button type=\"submit\" class=\"btn btn-primary\">Save</button>\n  </form>\n</div>\n{{{{ footer }}}}\n"
            ),
        ),
        (
            format!("{code}/catalog/controller/module/{c}.php"),
            format!(
                "<?php\nnamespace Opencart\\Catalog\\Controller\\Extension\\{class}\\Module;\n\nclass {class} extends \\Opencart\\System\\Engine\\Controller {{\n\
\tpublic function index(): string {{\n\t\tif (!$this->config->get('module_{c}_status')) {{\n\t\t\treturn '';\n\t\t}}\n\t\treturn '';\n\t}}\n}}\n"
            ),
        ),
    ]
}

/// PrestaShop 8 module with a configuration form (token-checked, validated, escaped by Smarty).
pub fn prestashop_module(code: &str, name: &str) -> Vec<(String, String)> {
    let c: String = code.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    let class = pascal(code);
    let up = c.to_ascii_uppercase();
    vec![
        (
            format!("{c}/{c}.php"),
            format!(
                "<?php\nif (!defined('_PS_VERSION_')) {{\n    exit;\n}}\n\nclass {class} extends Module\n{{\n    public function __construct()\n    {{\n\
        $this->name = '{c}';\n        $this->tab = 'front_office_features';\n        $this->version = '1.0.0';\n        $this->author = '';\n\
        $this->need_instance = 0;\n        $this->ps_versions_compliancy = ['min' => '8.0.0', 'max' => _PS_VERSION_];\n        $this->bootstrap = true;\n\
        parent::__construct();\n        $this->displayName = $this->trans('{name}', [], 'Modules.{class}.Admin');\n\
        $this->description = $this->trans('{name}', [], 'Modules.{class}.Admin');\n    }}\n\n\
    public function install()\n    {{\n        return parent::install() && $this->registerHook('displayHeader') && Configuration::updateValue('{up}_TEXT', '');\n    }}\n\n\
    public function uninstall()\n    {{\n        return Configuration::deleteByName('{up}_TEXT') && parent::uninstall();\n    }}\n\n\
    public function getContent()\n    {{\n        $out = '';\n        if (Tools::isSubmit('submit{class}')) {{\n\
            $text = (string) Tools::getValue('{up}_TEXT');\n            if (!Validate::isGenericName($text)) {{\n\
                $out .= $this->displayError($this->trans('Invalid value', [], 'Modules.{class}.Admin'));\n            }} else {{\n\
                Configuration::updateValue('{up}_TEXT', $text);\n                $out .= $this->displayConfirmation($this->trans('Settings updated', [], 'Admin.Notifications.Success'));\n            }}\n        }}\n\
        $helper = new HelperForm();\n        $helper->module = $this;\n        $helper->name_controller = $this->name;\n\
        $helper->token = Tools::getAdminTokenLite('AdminModules');\n        $helper->currentIndex = AdminController::$currentIndex . '&configure=' . $this->name;\n\
        $helper->submit_action = 'submit{class}';\n        $helper->fields_value['{up}_TEXT'] = Configuration::get('{up}_TEXT');\n\
        $form = [['form' => ['legend' => ['title' => $this->displayName], 'input' => [['type' => 'text', 'label' => 'Text', 'name' => '{up}_TEXT']], 'submit' => ['title' => $this->trans('Save', [], 'Admin.Actions')]]]];\n\
        return $out . $helper->generateForm($form);\n    }}\n\n\
    public function hookDisplayHeader($params)\n    {{\n        return '';\n    }}\n}}\n"
            ),
        ),
        (format!("{c}/index.php"), "<?php\nheader('Location: ../');\nexit;\n".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_folders_and_pages() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("system")).unwrap();
        std::fs::create_dir_all(d.path().join("catalog")).unwrap();
        std::fs::create_dir_all(d.path().join("admin")).unwrap();
        std::fs::write(d.path().join("system/startup.php"), "<?php").unwrap();
        std::fs::write(d.path().join("admin/index.php"), "<?php define('VERSION', '4.0.2.3');").unwrap();
        let found = detect_dir(d.path());
        assert_eq!(found[0].platform, "opencart");
        assert_eq!(found[0].version.as_deref(), Some("4.0.2.3"));
        let html = r#"<script src="https://static.ticimax.cloud/x.js"></script>"#;
        let p = detect_html(html);
        assert_eq!(p[0].platform, "ticimax");
        assert!(!p[0].modules);
        assert_eq!(detect_html(r#"<link href="/wp-content/themes/x/style.css"><meta name="generator" content="WordPress 6.6"><body class="woocommerce">"#).len(), 2);
    }

    #[test]
    fn module_skeletons() {
        let oc = opencart_module("kargo-takip", "Kargo Takip");
        assert!(oc[1].1.contains("namespace Opencart\\Admin\\Controller\\Extension\\KargoTakip\\Module;"));
        assert!(oc[1].1.contains("hasPermission('modify'"));
        let ps = prestashop_module("kargo-takip", "Kargo Takip");
        assert!(ps[0].0 == "kargotakip/kargotakip.php" && ps[0].1.contains("Validate::isGenericName"));
    }
}
