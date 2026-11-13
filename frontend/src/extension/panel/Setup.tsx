// Kurulum (§18.1, §18.4): bağlantı yolu (native / WebSocket), eşleştirme kodu, Kor'u başlat, hata ayıklayıcı izni
// (tam sayfa ekran görüntüsü ve izinli JS için isteğe bağlı), alan adı listeleri (yalnızca gösterim; değiştirmek
// `kor chrome allow|deny` ya da masaüstü ayarları).
import { useEffect, useState } from 'react'
import { ext } from '../ext.ts'
import { call, client, type Status } from './bus.ts'
import { t } from './i18n.ts'

export function Setup({ status }: { status: Status }) {
  const [code, setCode] = useState('')
  const [debuggerOn, setDebuggerOn] = useState(false)
  const [domains, setDomains] = useState<{ allow: string[]; deny: string[] } | null>(null)
  const [msg, setMsg] = useState('')
  const [remote, setRemote] = useState({ server: '', name: '', token: '' })

  useEffect(() => {
    void call<boolean>('debugger').then(setDebuggerOn, () => setDebuggerOn(false))
    if (status.paired) void client<{ allow: string[]; deny: string[] }>('policy').then(setDomains, () => setDomains(null))
  }, [status.paired])

  const pair = () => {
    setMsg('')
    void call('pair', { code }).then(() => setCode(''), (e: Error) => setMsg(e.message))
  }
  const saveRemote = () => {
    setMsg('')
    void call('remote', remote).then(() => {
      setRemote((r) => ({ ...r, token: '' }))
      setMsg(t('Uzak bağlantı kaydedildi.'))
    }, (e: Error) => setMsg(e.message))
  }
  const launch = () => void call('launch').then(() => setMsg(t('Kor başlatılıyor…')), (e: Error) => setMsg(e.message))
  const toggleDebugger = (on: boolean) => {
    // İzin isteği kullanıcı hareketiyle, eklenti sayfasından yapılmalı.
    const p = ext().permissions
    void (on ? p.request({ permissions: ['debugger'] }) : Promise.resolve(false)).then(setDebuggerOn, () => setDebuggerOn(false))
  }
  const mode = status.mode === 'native' ? t('yerel ana bilgisayar') : status.mode === 'ws' ? t('yerel WebSocket') : status.mode === 'remote' ? t('takım sunucusu üzerinden') : t('bağlantı yok')

  return (
    <section className="setup" aria-label={t('Kurulum')}>
      <p>
        {t('Bağlantı')}: {mode} · {status.core ? t('Kor çalışıyor') : t('Kor çalışmıyor')} · {status.paired ? t('eşleşti') : t('eşleşmedi')}
      </p>
      {status.mode === 'ws' ? <p className="note">{t('Yerel ana bilgisayar kayıtlı değil; birincil yol için terminalde: kor chrome install')}</p> : null}
      {!status.paired ? (
        <div className="row">
          <input
            type="text"
            aria-label={t('Eşleştirme kodu')}
            placeholder={t('Eşleştirme kodu (kor chrome pair)')}
            autoComplete="off"
            spellCheck={false}
            value={code}
            onChange={(e) => setCode(e.target.value)}
          />
          <button type="button" className="primary" onClick={pair} disabled={code.trim().length < 8}>
            {t('Eşleştir')}
          </button>
        </div>
      ) : null}
      {!status.paired ? (
        <details>
          <summary>{t('Başka bir makinedeki Kor (takım sunucusu)')}</summary>
          <div className="row">
            <input type="url" aria-label={t('Takım sunucusu')} placeholder="https://kor.example.com" autoComplete="off" spellCheck={false} value={remote.server} onChange={(e) => setRemote({ ...remote, server: e.target.value })} />
            <input type="text" aria-label={t('Makine adı')} placeholder={t('Makine adı')} autoComplete="off" spellCheck={false} value={remote.name} onChange={(e) => setRemote({ ...remote, name: e.target.value })} />
            <input type="password" aria-label={t('Üye belirteci')} placeholder={t('Üye belirteci')} autoComplete="off" value={remote.token} onChange={(e) => setRemote({ ...remote, token: e.target.value })} />
            <button type="button" onClick={saveRemote}>
              {t('Kaydet')}
            </button>
          </div>
          <p className="note">{t('O makinede kor peer serve çalışmalı; sunucu yalnızca iletileri aktarır, eşleştirme yine bu koddan yapılır.')}</p>
        </details>
      ) : null}
      {!status.core && status.canLaunch ? (
        <button type="button" onClick={launch}>
          {t('Kor\'u başlat')}
        </button>
      ) : null}
      <label className="sw">
        <input type="checkbox" checked={debuggerOn} onChange={(e) => toggleDebugger(e.target.checked)} />
        {t('Hata ayıklayıcı izni (tam sayfa ekran görüntüsü, izinli JS)')}
      </label>
      {domains ? (
        <p className="note" translate="no">
          {t('İzinli')}: {domains.allow.join(', ') || t('hepsi')} · {t('Engelli')}: {domains.deny.join(', ') || '—'}
        </p>
      ) : null}
      <p className="note">{t('Kor yalnızca kendi sekme grubunda çalışır; her işlem onay ister; parola ve kart alanlarına yazmaz, formu siz gönderirsiniz.')}</p>
      {status.error ? <p className="err">{status.error}</p> : null}
      {msg ? <p className="note">{msg}</p> : null}
    </section>
  )
}
