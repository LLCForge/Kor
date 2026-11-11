// [g3] feature 20: Kod bloğundaki kutu-çizgi ASCII şemasını monospace, hizalı ve kaydırılabilir gösterir.
//      ```ascii etiketiyle ya da etiketsiz bir blok ASCII şemaya benzediğinde (Markdown.tsx) kullanılır.
import { CopyButton } from './CopyButton.tsx'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'

export function AsciiDiagram({ code }: RichBlockProps) {
  return (
    <div className="code-block g3-ascii">
      <div className="code-head">
        <span className="g3-code-lang">ASCII şema</span>
        <div className="g3-code-tools">
          <CopyButton className="code-copy" label="Şemayı kopyala" getText={() => code} />
        </div>
      </div>
      <pre className="g3-ascii-art" role="img" aria-label="ASCII diyagramı">
        {code}
      </pre>
    </div>
  )
}

registerRichBlock('ascii', AsciiDiagram)
