// Yan panel metinleri: kaynak Türkçe (masaüstüyle aynı gettext yaklaşımı), İngilizce ve Fransızca sözlük.
// Dil tarayıcının arayüz dilidir. Kullanıcı/model içeriği çevrilmez (`translate="no"`).

type Dict = Record<string, string>

const en: Dict = {
  'takım sunucusu üzerinden': 'through the team server', 'Başka bir makinedeki Kor (takım sunucusu)': 'Kor on another machine (team server)',
  'Takım sunucusu': 'Team server', 'Makine adı': 'Machine name', 'Üye belirteci': 'Member token', 'Uzak bağlantı kaydedildi.': 'Remote connection saved.',
  'O makinede kor peer serve çalışmalı; sunucu yalnızca iletileri aktarır, eşleştirme yine bu koddan yapılır.': 'That machine must run kor peer serve; the server only forwards messages, and pairing still uses the code.',
  'Ad': 'Name', 'Ad sunucusu 1': 'Name server 1', 'Ad sunucusu 2': 'Name server 2', 'Ad sunucusu 3': 'Name server 3', 'Ad sunucusu 4': 'Name server 4',
  'Adım': 'Step', 'Adım ekran görüntüsü': 'Step screenshot', 'Adımlar': 'Steps', 'Alan': 'Field', 'Alan adı': 'Domain',
  'Alanları kontrol edip formu siz gönderin.': 'Check the fields and submit the form yourself.', 'Alt ad': 'Subdomain', 'Ayarlar': 'Settings',
  'Bağlam': 'Context', 'Bağlamı kaldır': 'Remove context', 'Bağlantı': 'Connection', 'Bileşen seç': 'Pick a component',
  'Bu dosyayı kabul et': 'Accept this file', 'Bu dosyayı yaz': 'Write this file', 'Bu görevin maliyeti': 'Cost of this task', 'Bu oturum': 'This session',
  'Bu oturumda henüz kullanım yok.': 'No usage in this session yet.',
  'Bu panel için bağlı bir Kor sunucu profili yok. Masaüstünde Web siteleri > Sunucu bağla ya da: kor host connect': 'No Kor server profile is linked to this panel. In the desktop app: Websites > Connect server, or: kor host connect',
  "Bu sayfayı Kor'a ver": 'Give this page to Kor', 'Bu sayfayı projemde kullan': 'Use this page in my project', "Bu sekmeyi Kor'a ver": 'Give this tab to Kor',
  'Bulunamadı': 'Not found', 'Bunu projemde nasıl kullanırım?': 'How do I use this in my project?', 'Bunu projemde yap': 'Build this in my project',
  'Değer': 'Value', 'Değiştir': 'Change', 'Değiştirdiğiniz değer yok.': 'You changed no values.', 'Dokümantasyon okuyucu': 'Documentation reader',
  'Doldur (göndermez)': 'Fill (does not submit)', 'Dolduruldu': 'Filled', 'Dolduruluyor…': 'Filling…', 'Doğrulama': 'Verification',
  'Doğrulama başarısız: değişiklik geri alındı.': 'Verification failed: the change was rolled back.', 'Durdur': 'Stop', 'Düzeltmelerimi öğren': 'Learn my corrections',
  'E-posta adresi': 'Email address', 'Ekran görüntüsü': 'Screenshot', 'Ekran okunuyor…': 'Reading the screen…', 'Engelli': 'Blocked',
  'Envanter okunuyor…': 'Reading the inventory…', 'Etki': 'Impact', 'Eşleşen alan yok.': 'No matching fields.', 'Eşleşmeyen': 'Unmatched', 'Eşleştir': 'Pair',
  'Eşleştirme kodu': 'Pairing code', 'Eşleştirme kodu (kor chrome pair)': 'Pairing code (kor chrome pair)', 'Form doldurucu': 'Form filler', 'Formu doldur': 'Fill the form',
  'Formu kontrol edip gönder düğmesine siz basın.': 'Check the form and press submit yourself.', 'Formu oku ve eşleştir': 'Read and match the form',
  'Geri alınamaz işlem': 'Irreversible action', 'Gönder': 'Send', 'Görseller ve logolar kopyalanmaz, yer tutucuyla değiştirildi': 'Images and logos are not copied; they were replaced with placeholders',
  'Hata ayıklayıcı izni (tam sayfa ekran görüntüsü, izinli JS)': 'Debugger permission (full-page screenshots, approved JS)', 'Hedef': 'Target', 'Her zaman': 'Always',
  'Kaydet': 'Save', 'Klasör': 'Folder', 'Kod üretiliyor…': 'Generating code…', 'Komut': 'Command', 'Kor başlatılıyor…': 'Starting Kor…',
  'Kor bu alanlara yazmaz (parola, kart, kod)': 'Kor never types into these fields (password, card, code)', 'Kor profili': 'Kor profile', 'Kor profili yok': 'No Kor profile',
  'Kor yalnızca kendi grubundaki sekmelerde çalışır; bu sekmeyi göreve verir.': 'Kor works only in tabs of its own group; this hands the tab to the task.',
  'Kor yalnızca kendi sekme grubunda çalışır; her işlem onay ister; parola ve kart alanlarına yazmaz, formu siz gönderirsiniz.': 'Kor works only in its own tab group; every action asks; it never types passwords or card data, and you submit forms.',
  'Kor çalışmıyor': 'Kor is not running', "Kor çalışmıyor — Kor masaüstünü açın ya da bir terminalde Kor'u başlatın.": 'Kor is not running — open the Kor desktop app or start Kor in a terminal.',
  'Kor çalışıyor': 'Kor is running', "Kor'a bir görev yazın; CLI ve masaüstündeki görevler burada da görünür.": 'Write a task for Kor; tasks from the CLI and the desktop show up here too.',
  "Kor'a yaz… (Enter gönderir, / komutlar)": 'Write to Kor… (Enter sends, / commands)', "Kor'u başlat": 'Start Kor', 'Kullanıcı': 'User', 'Kurulum': 'Setup',
  'Maliyet': 'Cost', 'Mesaj': 'Message', 'Model': 'Model', 'Model değişti': 'Model changed', 'Okunuyor…': 'Reading…', 'Onayla ve uygula': 'Approve and apply',
  'Oturum': 'Session', 'PHP sürümü (ör. 8.3)': 'PHP version (e.g. 8.3)', 'Panel': 'Panel', 'Panel komutları': 'Panel commands', 'Parola Kor kasasında': 'Password is in the Kor vault',
  'Plan ve etki': 'Plan and impact', 'Planı göster': 'Show the plan', 'Profil': 'Profile', 'Profil güncellendi.': 'Profile updated.',
  'Profil yok: Kor masaüstü > Ayarlar > Tarayıcı eklentisi ya da kor chrome profile set': 'No profile: Kor desktop > Settings > Browser extension, or kor chrome profile set',
  'Profile kaydedilsin mi?': 'Save to the profile?', 'Proje': 'Project', 'Proje grafiği': 'Project graph', 'Projenin yığını': "The project's stack", 'Reddet': 'Reject',
  'Sayfa': 'Page', 'Sayfa metni': 'Page text', 'Sayfa → kod': 'Page → code', 'Sayfada bir bileşene tıklayın (Esc: iptal)': 'Click a component on the page (Esc: cancel)',
  'Sekme Kor grubuna verildi': "Tab added to Kor's group", 'Serbest metin taslakları': 'Free-text drafts', 'Seçilen bileşen': 'Picked component',
  'Seçilen dosyaları yaz': 'Write the selected files', 'Seçili metin': 'Selected text', 'Sohbet': 'Chat', 'Soru': 'Question', 'TTL': 'TTL',
  'Taslak için ipucu': 'Hint for drafts', 'Taslak için ipucu (ör. deneyimim)': 'Hint for drafts (e.g. my experience)', 'Tek tıkla başlatma için: kor chrome install': 'For one-click start: kor chrome install',
  'Test': 'Test', 'Uygula (dosya başına onayla)': 'Apply (approve each file)', 'Uygulandı': 'Applied', 'Uygulanıyor ve doğrulanıyor…': 'Applying and verifying…',
  'Vazgeç': 'Cancel', 'Yazıldı': 'Written', 'Yeni görev': 'New task', 'Yenile': 'Refresh',
  'Yerel ana bilgisayar kayıtlı değil; birincil yol için terminalde: kor chrome install': 'The native host is not registered; for the primary connection run in a terminal: kor chrome install',
  'Yükleniyor…': 'Loading…', 'Zaman (*/15 * * * *)': 'Schedule (*/15 * * * *)', 'bağlantı yok': 'not connected', 'bağlı': 'connected', 'bağlı değil': 'not connected',
  'doküman': 'docs', 'dosya': 'files', 'düğüm': 'nodes', 'ekran görüntüsü': 'screenshot', 'eşleşmedi': 'not paired', 'eşleşti': 'paired', 'geri alınamaz': 'irreversible',
  'girdi': 'input', 'görsel karşılaştırma yok': 'no visual comparison', 'hepsi': 'all', 'kenar': 'edges', 'orijinale yakınlık': 'similarity to the original', 'proje': 'project',
  'sayfa': 'page', 'seçim': 'selection', 'silinir': 'deleted', 'sürüm uyuşmazlığı: kod projenin sürümüne göre yazıldı': "version mismatch: the code targets the project's version",
  'taslak': 'draft', 'topluluk': 'communities', 'yazılmadı': 'not written', 'yeni dosya': 'new file', 'yerel WebSocket': 'local WebSocket', 'yerel ana bilgisayar': 'native host',
  'Çalışan göreve mesaj ekle… (Enter)': 'Add a message to the running task… (Enter)', 'Çalışıyor…': 'Working…', 'çıktı': 'output', 'önbellek': 'cache',
  'İlgili yerler': 'Related places', 'İzin ver': 'Allow', 'İzinli': 'Allowed', 'İçerik / IPv4': 'Content / IPv4', 'Şablon yok.': 'No templates.', 'Şablonlar': 'Templates',
  '… (kısaltıldı)': '… (truncated)',
  // §20.3 galeri
  'Galeri': 'Gallery', 'Galeri boş: Kor ile üretilen görsel, video ve sesler burada görünür.': 'The gallery is empty: images, videos and audio made with Kor show up here.',
  'Aç': 'Open', 'Kopyala': 'Copy', 'İndir': 'Download', 'Kapat': 'Close', 'Kopyalandı': 'Copied', 'Kopyalanamadı.': 'Could not copy.',
  'Medya alınamadı.': 'Could not load the media.', 'Daha fazla': 'More',
}

const fr: Dict = {
  'takım sunucusu üzerinden': "via le serveur d'équipe", 'Başka bir makinedeki Kor (takım sunucusu)': "Kor sur une autre machine (serveur d'équipe)",
  'Takım sunucusu': "Serveur d'équipe", 'Makine adı': 'Nom de la machine', 'Üye belirteci': 'Jeton de membre', 'Uzak bağlantı kaydedildi.': 'Connexion distante enregistrée.',
  'O makinede kor peer serve çalışmalı; sunucu yalnızca iletileri aktarır, eşleştirme yine bu koddan yapılır.': "Cette machine doit lancer kor peer serve ; le serveur ne fait que relayer les messages, l'appairage passe toujours par le code.",
  'Ad': 'Nom', 'Ad sunucusu 1': 'Serveur de noms 1', 'Ad sunucusu 2': 'Serveur de noms 2', 'Ad sunucusu 3': 'Serveur de noms 3', 'Ad sunucusu 4': 'Serveur de noms 4',
  'Adım': 'Étape', 'Adım ekran görüntüsü': "Capture de l'étape", 'Adımlar': 'Étapes', 'Alan': 'Champ', 'Alan adı': 'Domaine',
  'Alanları kontrol edip formu siz gönderin.': 'Vérifiez les champs et envoyez le formulaire vous-même.', 'Alt ad': 'Sous-domaine', 'Ayarlar': 'Réglages',
  'Bağlam': 'Contexte', 'Bağlamı kaldır': 'Retirer le contexte', 'Bağlantı': 'Connexion', 'Bileşen seç': 'Choisir un composant',
  'Bu dosyayı kabul et': 'Accepter ce fichier', 'Bu dosyayı yaz': 'Écrire ce fichier', 'Bu görevin maliyeti': 'Coût de cette tâche', 'Bu oturum': 'Cette session',
  'Bu oturumda henüz kullanım yok.': "Pas encore d'utilisation dans cette session.",
  'Bu panel için bağlı bir Kor sunucu profili yok. Masaüstünde Web siteleri > Sunucu bağla ya da: kor host connect': "Aucun profil de serveur Kor n'est lié à ce panneau. Dans l'application : Sites web > Connecter un serveur, ou : kor host connect",
  "Bu sayfayı Kor'a ver": 'Donner cette page à Kor', 'Bu sayfayı projemde kullan': 'Utiliser cette page dans mon projet', "Bu sekmeyi Kor'a ver": 'Donner cet onglet à Kor',
  'Bulunamadı': 'Introuvable', 'Bunu projemde nasıl kullanırım?': "Comment l'utiliser dans mon projet ?", 'Bunu projemde yap': 'Le réaliser dans mon projet',
  'Değer': 'Valeur', 'Değiştir': 'Changer', 'Değiştirdiğiniz değer yok.': "Vous n'avez modifié aucune valeur.", 'Dokümantasyon okuyucu': 'Lecteur de documentation',
  'Doldur (göndermez)': "Remplir (sans envoyer)", 'Dolduruldu': 'Rempli', 'Dolduruluyor…': 'Remplissage…', 'Doğrulama': 'Vérification',
  'Doğrulama başarısız: değişiklik geri alındı.': 'Échec de la vérification : la modification a été annulée.', 'Durdur': 'Arrêter', 'Düzeltmelerimi öğren': 'Apprendre mes corrections',
  'E-posta adresi': 'Adresse e-mail', 'Ekran görüntüsü': "Capture d'écran", 'Ekran okunuyor…': "Lecture de l'écran…", 'Engelli': 'Bloqués',
  'Envanter okunuyor…': "Lecture de l'inventaire…", 'Etki': 'Impact', 'Eşleşen alan yok.': 'Aucun champ correspondant.', 'Eşleşmeyen': 'Sans correspondance', 'Eşleştir': 'Appairer',
  'Eşleştirme kodu': "Code d'appairage", 'Eşleştirme kodu (kor chrome pair)': "Code d'appairage (kor chrome pair)", 'Form doldurucu': 'Remplisseur de formulaires', 'Formu doldur': 'Remplir le formulaire',
  'Formu kontrol edip gönder düğmesine siz basın.': "Vérifiez le formulaire et appuyez vous-même sur envoyer.", 'Formu oku ve eşleştir': 'Lire et associer le formulaire',
  'Geri alınamaz işlem': 'Action irréversible', 'Gönder': 'Envoyer', 'Görseller ve logolar kopyalanmaz, yer tutucuyla değiştirildi': 'Les images et logos ne sont pas copiés ; ils ont été remplacés par des espaces réservés',
  'Hata ayıklayıcı izni (tam sayfa ekran görüntüsü, izinli JS)': 'Autorisation du débogueur (captures pleine page, JS approuvé)', 'Hedef': 'Cible', 'Her zaman': 'Toujours',
  'Kaydet': 'Enregistrer', 'Klasör': 'Dossier', 'Kod üretiliyor…': 'Génération du code…', 'Komut': 'Commande', 'Kor başlatılıyor…': 'Démarrage de Kor…',
  'Kor bu alanlara yazmaz (parola, kart, kod)': "Kor n'écrit jamais dans ces champs (mot de passe, carte, code)", 'Kor profili': 'Profil Kor', 'Kor profili yok': 'Aucun profil Kor',
  'Kor yalnızca kendi grubundaki sekmelerde çalışır; bu sekmeyi göreve verir.': "Kor n'agit que dans les onglets de son groupe ; ceci confie l'onglet à la tâche.",
  'Kor yalnızca kendi sekme grubunda çalışır; her işlem onay ister; parola ve kart alanlarına yazmaz, formu siz gönderirsiniz.': "Kor n'agit que dans son groupe d'onglets ; chaque action demande confirmation ; il ne saisit jamais de mot de passe ni de carte, et c'est vous qui envoyez les formulaires.",
  'Kor çalışmıyor': "Kor n'est pas lancé", "Kor çalışmıyor — Kor masaüstünü açın ya da bir terminalde Kor'u başlatın.": "Kor n'est pas lancé — ouvrez l'application Kor ou lancez Kor dans un terminal.",
  'Kor çalışıyor': 'Kor est lancé', "Kor'a bir görev yazın; CLI ve masaüstündeki görevler burada da görünür.": "Écrivez une tâche pour Kor ; les tâches du CLI et de l'application s'affichent aussi ici.",
  "Kor'a yaz… (Enter gönderir, / komutlar)": 'Écrire à Kor… (Entrée envoie, / commandes)', "Kor'u başlat": 'Démarrer Kor', 'Kullanıcı': 'Utilisateur', 'Kurulum': 'Installation',
  'Maliyet': 'Coût', 'Mesaj': 'Message', 'Model': 'Modèle', 'Model değişti': 'Modèle changé', 'Okunuyor…': 'Lecture…', 'Onayla ve uygula': 'Approuver et appliquer',
  'Oturum': 'Session', 'PHP sürümü (ör. 8.3)': 'Version de PHP (ex. 8.3)', 'Panel': 'Panneau', 'Panel komutları': 'Commandes du panneau', 'Parola Kor kasasında': 'Le mot de passe est dans le coffre Kor',
  'Plan ve etki': 'Plan et impact', 'Planı göster': 'Afficher le plan', 'Profil': 'Profil', 'Profil güncellendi.': 'Profil mis à jour.',
  'Profil yok: Kor masaüstü > Ayarlar > Tarayıcı eklentisi ya da kor chrome profile set': "Aucun profil : application Kor > Réglages > Extension de navigateur, ou kor chrome profile set",
  'Profile kaydedilsin mi?': 'Enregistrer dans le profil ?', 'Proje': 'Projet', 'Proje grafiği': 'Graphe du projet', 'Projenin yığını': 'La pile du projet', 'Reddet': 'Refuser',
  'Sayfa': 'Page', 'Sayfa metni': 'Texte de la page', 'Sayfa → kod': 'Page → code', 'Sayfada bir bileşene tıklayın (Esc: iptal)': 'Cliquez sur un composant de la page (Échap : annuler)',
  'Sekme Kor grubuna verildi': 'Onglet ajouté au groupe de Kor', 'Serbest metin taslakları': 'Brouillons de texte libre', 'Seçilen bileşen': 'Composant choisi',
  'Seçilen dosyaları yaz': 'Écrire les fichiers choisis', 'Seçili metin': 'Texte sélectionné', 'Sohbet': 'Discussion', 'Soru': 'Question', 'TTL': 'TTL',
  'Taslak için ipucu': 'Indication pour les brouillons', 'Taslak için ipucu (ör. deneyimim)': 'Indication pour les brouillons (ex. mon expérience)', 'Tek tıkla başlatma için: kor chrome install': 'Pour le démarrage en un clic : kor chrome install',
  'Test': 'Test', 'Uygula (dosya başına onayla)': 'Appliquer (approuver chaque fichier)', 'Uygulandı': 'Appliqué', 'Uygulanıyor ve doğrulanıyor…': 'Application et vérification…',
  'Vazgeç': 'Annuler', 'Yazıldı': 'Écrit', 'Yeni görev': 'Nouvelle tâche', 'Yenile': 'Actualiser',
  'Yerel ana bilgisayar kayıtlı değil; birincil yol için terminalde: kor chrome install': "L'hôte natif n'est pas enregistré ; pour la connexion principale, exécutez dans un terminal : kor chrome install",
  'Yükleniyor…': 'Chargement…', 'Zaman (*/15 * * * *)': 'Planification (*/15 * * * *)', 'bağlantı yok': 'non connecté', 'bağlı': 'connecté', 'bağlı değil': 'non connecté',
  'doküman': 'docs', 'dosya': 'fichiers', 'düğüm': 'nœuds', 'ekran görüntüsü': "capture d'écran", 'eşleşmedi': 'non appairé', 'eşleşti': 'appairé', 'geri alınamaz': 'irréversible',
  'girdi': 'entrée', 'görsel karşılaştırma yok': 'pas de comparaison visuelle', 'hepsi': 'tous', 'kenar': 'arêtes', 'orijinale yakınlık': "proximité avec l'original", 'proje': 'projet',
  'sayfa': 'page', 'seçim': 'sélection', 'silinir': 'supprimé', 'sürüm uyuşmazlığı: kod projenin sürümüne göre yazıldı': 'versions différentes : le code vise la version du projet',
  'taslak': 'brouillon', 'topluluk': 'communautés', 'yazılmadı': 'non écrits', 'yeni dosya': 'nouveau fichier', 'yerel WebSocket': 'WebSocket local', 'yerel ana bilgisayar': 'hôte natif',
  'Çalışan göreve mesaj ekle… (Enter)': 'Ajouter un message à la tâche en cours… (Entrée)', 'Çalışıyor…': 'En cours…', 'çıktı': 'sortie', 'önbellek': 'cache',
  'İlgili yerler': 'Endroits liés', 'İzin ver': 'Autoriser', 'İzinli': 'Autorisés', 'İçerik / IPv4': 'Contenu / IPv4', 'Şablon yok.': 'Aucun modèle.', 'Şablonlar': 'Modèles',
  '… (kısaltıldı)': '… (tronqué)',
  // §20.3 galeri
  'Galeri': 'Galerie', 'Galeri boş: Kor ile üretilen görsel, video ve sesler burada görünür.': 'La galerie est vide : les images, vidéos et sons créés avec Kor apparaissent ici.',
  'Aç': 'Ouvrir', 'Kopyala': 'Copier', 'İndir': 'Télécharger', 'Kapat': 'Fermer', 'Kopyalandı': 'Copié', 'Kopyalanamadı.': 'Copie impossible.',
  'Medya alınamadı.': 'Impossible de charger le média.', 'Daha fazla': 'Plus',
}

export const DICTS: Record<string, Dict> = { en, fr }

let dict: Dict | null = null

export function setLanguage(lang: string): void {
  dict = DICTS[lang.slice(0, 2)] ?? null
}

/** Türkçe kaynak metni arayüz diline çevirir; sözlükte yoksa kaynak döner. */
export function t(tr: string): string {
  return dict?.[tr] ?? tr
}

export function applyLanguage(): void {
  const lang = (globalThis.navigator?.language ?? 'tr').slice(0, 2)
  setLanguage(lang)
  document.documentElement.lang = DICTS[lang] ? lang : 'tr'
}
