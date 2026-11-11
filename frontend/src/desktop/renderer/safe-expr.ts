// [g8] Güvenli aritmetik ifade değerlendirici (hesap makinesi #101 ve slider #103 için).
//      GÜVENLİK: eval / new Function KULLANILMAZ. Elle yazılmış özyinelemeli inişli ayrıştırıcı yalnızca
//      sayı, değişken, izinli fonksiyon ve + - * / % ^ ve parantez tanır; başka hiçbir JS çalıştırılmaz.
//      Girdi uzunluğu ve özyineleme derinliği sınırlıdır (ReDoS/yığın taşması önleme).

/** İfade kaynağı için üst sınır (DoS: devasa ifade ayrıştırılmaz). */
export const MAX_EXPR_LENGTH = 1000
const MAX_DEPTH = 64

/** İzin verilen fonksiyonlar (allowlist). Başka tanımlayıcı fonksiyon olarak çağrılamaz. */
const FUNCTIONS: Record<string, (args: number[]) => number> = {
  abs: ([x]) => Math.abs(x ?? Number.NaN),
  sqrt: ([x]) => Math.sqrt(x ?? Number.NaN),
  cbrt: ([x]) => Math.cbrt(x ?? Number.NaN),
  round: ([x]) => Math.round(x ?? Number.NaN),
  floor: ([x]) => Math.floor(x ?? Number.NaN),
  ceil: ([x]) => Math.ceil(x ?? Number.NaN),
  trunc: ([x]) => Math.trunc(x ?? Number.NaN),
  sign: ([x]) => Math.sign(x ?? Number.NaN),
  exp: ([x]) => Math.exp(x ?? Number.NaN),
  ln: ([x]) => Math.log(x ?? Number.NaN),
  log10: ([x]) => Math.log10(x ?? Number.NaN),
  log2: ([x]) => Math.log2(x ?? Number.NaN),
  sin: ([x]) => Math.sin(x ?? Number.NaN),
  cos: ([x]) => Math.cos(x ?? Number.NaN),
  tan: ([x]) => Math.tan(x ?? Number.NaN),
  pow: ([a, b]) => (a ?? Number.NaN) ** (b ?? Number.NaN),
  min: (args) => (args.length ? Math.min(...args) : Number.NaN),
  max: (args) => (args.length ? Math.max(...args) : Number.NaN),
  hypot: (args) => Math.hypot(...args),
  mod: ([a, b]) => (a ?? Number.NaN) % (b ?? Number.NaN),
  clamp: ([x, lo, hi]) => Math.min(Math.max(x ?? Number.NaN, lo ?? Number.NaN), hi ?? Number.NaN),
}

/** Değişken olarak çağrılabilen sabitler (kullanıcı değişkenleri bunları gölgeler). */
const CONSTANTS: Record<string, number> = { pi: Math.PI, e: Math.E, tau: Math.PI * 2 }

type Token =
  | { t: 'num'; v: number }
  | { t: 'id'; v: string }
  | { t: 'op'; v: '+' | '-' | '*' | '/' | '%' | '^' | '(' | ')' | ',' }

const NUMBER = /^(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/
const IDENT = /^[A-Za-z_]\w*/

/** Ham metni belirteçlere ayırır. Tanınmayan karakterde hata fırlatır. */
function tokenize(src: string): Token[] {
  const tokens: Token[] = []
  let i = 0
  while (i < src.length) {
    const ch = src[i]!
    if (ch === ' ' || ch === '\t' || ch === '\n' || ch === '\r') {
      i++
      continue
    }
    if ('+-*/%^(),'.includes(ch)) {
      tokens.push({ t: 'op', v: ch as Extract<Token, { t: 'op' }>['v'] })
      i++
      continue
    }
    const rest = src.slice(i)
    const num = NUMBER.exec(rest)
    if (num) {
      tokens.push({ t: 'num', v: Number(num[0]) })
      i += num[0].length
      continue
    }
    const id = IDENT.exec(rest)
    if (id) {
      tokens.push({ t: 'id', v: id[0] })
      i += id[0].length
      continue
    }
    throw new Error(`Geçersiz karakter: ${ch}`)
  }
  return tokens
}

/**
 * Belirteç akışını özyinelemeli iniş ile değerlendirir.
 * Öncelik: + -  <  * / %  <  birli -/+  <  ^ (sağ birleşmeli). Üs birli işareti kapsar: -2^2 = -4, 2^3^2 = 512.
 */
class Parser {
  private pos = 0
  private depth = 0
  private readonly tokens: Token[]
  private readonly vars: Record<string, number>
  constructor(tokens: Token[], vars: Record<string, number>) {
    this.tokens = tokens
    this.vars = vars
  }

  private peek(): Token | undefined {
    return this.tokens[this.pos]
  }

  private isOp(v: string): boolean {
    const tok = this.peek()
    return tok?.t === 'op' && tok.v === v
  }

  private enter(): void {
    if (++this.depth > MAX_DEPTH) throw new Error('İfade çok derin iç içe')
  }

  parse(): number {
    const value = this.expr()
    if (this.pos < this.tokens.length) throw new Error('Beklenmeyen simge')
    return value
  }

  private expr(): number {
    let value = this.term()
    while (this.isOp('+') || this.isOp('-')) {
      const op = (this.peek() as Extract<Token, { t: 'op' }>).v
      this.pos++
      const rhs = this.term()
      value = op === '+' ? value + rhs : value - rhs
    }
    return value
  }

  private term(): number {
    let value = this.factor()
    while (this.isOp('*') || this.isOp('/') || this.isOp('%')) {
      const op = (this.peek() as Extract<Token, { t: 'op' }>).v
      this.pos++
      const rhs = this.factor()
      value = op === '*' ? value * rhs : op === '/' ? value / rhs : value % rhs
    }
    return value
  }

  private factor(): number {
    this.enter()
    try {
      if (this.isOp('-')) {
        this.pos++
        return -this.factor()
      }
      if (this.isOp('+')) {
        this.pos++
        return this.factor()
      }
      return this.power()
    } finally {
      this.depth--
    }
  }

  private power(): number {
    const base = this.primary()
    if (this.isOp('^')) {
      this.pos++
      return base ** this.factor() // sağ birleşmeli; üs birli işaret alabilir
    }
    return base
  }

  private primary(): number {
    const tok = this.peek()
    if (!tok) throw new Error('İfade beklenmedik şekilde bitti')
    if (tok.t === 'num') {
      this.pos++
      return tok.v
    }
    if (tok.t === 'op' && tok.v === '(') {
      this.pos++
      const value = this.expr()
      if (!this.isOp(')')) throw new Error('Kapanış parantezi eksik')
      this.pos++
      return value
    }
    if (tok.t === 'id') {
      this.pos++
      if (this.isOp('(')) return this.call(tok.v)
      const lower = tok.v.toLowerCase()
      if (Object.prototype.hasOwnProperty.call(this.vars, tok.v)) return this.vars[tok.v]!
      if (Object.prototype.hasOwnProperty.call(CONSTANTS, lower)) return CONSTANTS[lower]!
      throw new Error(`Bilinmeyen değişken: ${tok.v}`)
    }
    throw new Error('Geçersiz ifade')
  }

  private call(name: string): number {
    const fn = FUNCTIONS[name.toLowerCase()]
    if (!fn) throw new Error(`Bilinmeyen fonksiyon: ${name}`)
    this.pos++ // '('
    const args: number[] = []
    if (!this.isOp(')')) {
      args.push(this.expr())
      while (this.isOp(',')) {
        this.pos++
        args.push(this.expr())
      }
    }
    if (!this.isOp(')')) throw new Error('Fonksiyon çağrısında kapanış parantezi eksik')
    this.pos++
    return fn(args)
  }
}

/**
 * İfadeyi güvenle değerlendirir. Geçersiz/bilinmeyen simgede ya da sonuç sonlu değilse Error fırlatır.
 * @param expr aritmetik ifade metni
 * @param vars değişken adı → sayı eşlemesi (hesap makinesi/slider girdileri)
 */
export function evaluateExpr(expr: string, vars: Record<string, number> = {}): number {
  if (expr.length > MAX_EXPR_LENGTH) throw new Error('İfade çok uzun')
  const tokens = tokenize(expr)
  if (tokens.length === 0) throw new Error('Boş ifade')
  const value = new Parser(tokens, vars).parse()
  if (!Number.isFinite(value)) throw new Error('Sonuç tanımsız (0 bölme veya taşma)')
  return value
}

/** İfadede geçen, bilinen sabit/fonksiyon olmayan tanımlayıcılar (hesap makinesi girdi alanlarını üretmek için). */
export function expressionVariables(expr: string): string[] {
  let tokens: Token[]
  try {
    tokens = tokenize(expr)
  } catch {
    return []
  }
  const seen = new Set<string>()
  for (let i = 0; i < tokens.length; i++) {
    const tok = tokens[i]!
    if (tok.t !== 'id') continue
    const next = tokens[i + 1]
    const isCall = next?.t === 'op' && next.v === '('
    if (isCall) continue // fonksiyon adı, değişken değil
    if (Object.prototype.hasOwnProperty.call(CONSTANTS, tok.v.toLowerCase())) continue
    seen.add(tok.v)
  }
  return [...seen]
}
