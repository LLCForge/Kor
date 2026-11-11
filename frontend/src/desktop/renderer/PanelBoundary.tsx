// Sağ panel hata sınırı: bir görünümün çizimde fırlattığı hata yalnızca o paneli düşürür, pencerenin tamamını değil.
import { Component, type ErrorInfo, type ReactNode } from 'react'
import { errorMessage } from './api.ts'

interface Props {
  /** Değişince (ör. başka görünüme geçildiğinde) hata durumu sıfırlanır. */
  resetKey: string
  children: ReactNode
}

interface State {
  error: unknown
  key: string
}

export class PanelBoundary extends Component<Props, State> {
  override state: State = { error: null, key: this.props.resetKey }

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error: error ?? new Error('Bilinmeyen hata') }
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    return props.resetKey === state.key ? null : { error: null, key: props.resetKey }
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error('[panel]', this.props.resetKey, error, info.componentStack)
  }

  override render(): ReactNode {
    if (this.state.error === null) return this.props.children
    return (
      <div className="panel-body" role="alert">
        <p className="ks-empty">
          Bu panel çizilirken bir hata oluştu: <code>{errorMessage(this.state.error)}</code>
        </p>
        <button type="button" className="btn small" onClick={() => this.setState({ error: null })}>
          Yeniden dene
        </button>
      </div>
    )
  }
}
