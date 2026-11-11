// "Beni çağır" (§28.4), tepsi (§27.1) ve PiP (§27.8) ayarları: kullanıcı config.toml `notify.*`, `tray.*`, `pip.*`.
import { useState } from 'react'
import useSWR from 'swr'
import type { NotificationCategory, ShellSettingKey, ShellSettings as Settings } from '../../ipc-shellui.ts'
import { errorMessage } from '../api.ts'
import { shellui } from './bridge.ts'
import { CATEGORY_LABEL, validQuietHours } from './state.ts'

const CATEGORIES: NotificationCategory[] = ['approval', 'question', 'done', 'error', 'budget', 'update']

export function ShellSettings() {
  const { data, error, isLoading, mutate } = useSWR<Settings>('shellui:settings', () => shellui.settings())
  const [problem, setProblem] = useState<string | null>(null)
  const [quiet, setQuiet] = useState<string | null>(null)

  const set = async (key: ShellSettingKey, value: boolean | number | string | string[]) => {
    setProblem(null)
    try {
      await mutate(await shellui.setSetting(key, value), { revalidate: false })
    } catch (err) {
      setProblem(errorMessage(err))
    }
  }

  if (isLoading) return <p className="ksh-muted" role="status">Ayarlar yükleniyor…</p>
  if (error || !data) {
    return (
      <p className="error-text" role="alert">
        Ayarlar alınamadı: {errorMessage(error)}
      </p>
    )
  }
  const n = data.notify
  const toggle = (key: ShellSettingKey, checked: boolean, label: string, hint?: string) => (
    <label className="ksh-toggle">
      <input type="checkbox" checked={checked} onChange={(e) => void set(key, e.target.checked)} data-kor-cmd="shellui.setSetting" />
      <span>
        {label}
        {hint && <span className="ksh-muted"> — {hint}</span>}
      </span>
    </label>
  )
  const quietValue = quiet ?? n.quietHours
  return (
    <div className="ksh-settings" data-kor-cmd="shellui.settings">
      <fieldset>
        <legend>Beni çağır</legend>
        {toggle('notify.toast', n.toast, 'İşletim sistemi bildirimi', 'tıklanınca ilgili sohbet açılır')}
        {toggle('notify.sound', n.sound, 'Kısa ses')}
        {toggle('notify.flash', n.flash, 'Görev çubuğunu yakıp söndür / Dock zıplat', 'onay ya da soru beklerken')}
        {toggle('notify.bell', n.bell, 'Terminalde BEL ve başlık', 'kor CLI/TUI onay beklerken')}
        <label className="ksh-field">
          <span>Boşta kalma süresi (sn)</span>
          <input
            type="number"
            min={0}
            max={3600}
            value={n.idleSecs}
            onChange={(e) => {
              const v = Number(e.target.value)
              if (Number.isInteger(v) && v >= 0 && v <= 3600) void set('notify.idle_secs', v)
            }}
            data-kor-cmd="shellui.setSetting"
          />
          <span className="ksh-muted">Pencere öndeyken ve bu süreden kısa zamandır kullanıyorsanız yalnızca sessiz vurgu; 0 her zaman bildirir.</span>
        </label>
        <label className="ksh-field">
          <span>Sessiz saatler</span>
          <input
            type="text"
            placeholder="22-08"
            value={quietValue}
            aria-invalid={!validQuietHours(quietValue)}
            onChange={(e) => setQuiet(e.target.value)}
            onBlur={() => {
              if (quiet === null) return
              if (validQuietHours(quiet)) void set('notify.quiet_hours', quiet.trim()).then(() => setQuiet(null))
              else setProblem('Sessiz saatler SS-SS biçiminde olmalı (ör. 22-08)')
            }}
            data-kor-cmd="shellui.setSetting"
          />
          <span className="ksh-muted">Bu saatlerde ses, BEL ve yanıp sönme yok; bildirim sessizce gelir.</span>
        </label>
      </fieldset>
      <fieldset>
        <legend>Kategoriler</legend>
        {CATEGORIES.map((c) => (
          <label key={c} className="ksh-toggle">
            <input
              type="checkbox"
              checked={!n.muted.includes(c)}
              onChange={(e) => void set('notify.muted', e.target.checked ? n.muted.filter((m) => m !== c) : [...n.muted, c])}
              data-kor-cmd="shellui.setSetting"
            />
            <span>{CATEGORY_LABEL[c]}</span>
          </label>
        ))}
        <p className="ksh-muted">Kapalı kategoriler yalnızca zil listesine yazılır.</p>
      </fieldset>
      <fieldset>
        <legend>Tepsi</legend>
        {toggle('tray.enabled', data.tray.enabled, 'Tepsi simgesi')}
        {toggle('tray.close_to_tray', data.tray.closeToTray, 'Pencere kapanınca tepside çalışmaya devam et')}
      </fieldset>
      <fieldset>
        <legend>Durum penceresi</legend>
        {toggle('pip.auto_show', data.pip.autoShow, 'Görev başlayınca otomatik göster', 'bitince 5 sn sonra kaybolur')}
        {toggle('pip.hide_fullscreen', data.pip.hideFullscreen, 'Tam ekran uygulamalarda gizle')}
        <label className="ksh-field">
          <span>Saydamlık</span>
          <input
            type="range"
            min={0.3}
            max={1}
            step={0.05}
            value={data.pip.opacity}
            onChange={(e) => void set('pip.opacity', Number(e.target.value))}
            data-kor-cmd="shellui.setSetting"
          />
        </label>
        <button type="button" className="btn small" onClick={() => void shellui.pip()} data-kor-cmd="shellui.pip">
          Durum penceresini aç/kapat
        </button>
      </fieldset>
      {problem && (
        <p className="error-text" role="alert">
          {problem}
        </p>
      )}
    </div>
  )
}
