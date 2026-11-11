import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'

export interface ProjectCommands {
  typecheck: string | null
  test: string | null
  build: string | null
  lint: string | null
}

export interface ProjectInfo extends ProjectCommands {
  /** ör. "Node.js (TypeScript)", "Python", ".NET" */
  kinds: string[]
  packageManager: string | null
}

/** Kök dizinden okunan girdi: dosya adı listesi + algılama için gereken küçük dosyaların içeriği. */
export interface ProjectFiles {
  names: ReadonlySet<string>
  read(name: string): string | null
}

const packageJsonSchema = z
  .object({
    packageManager: z.string().max(100).optional(),
    scripts: z.record(z.string(), z.string().max(2000)).optional(),
    dependencies: z.record(z.string(), z.unknown()).optional(),
    devDependencies: z.record(z.string(), z.unknown()).optional(),
  })
  .passthrough()

function nodeProject(files: ProjectFiles): ProjectInfo | null {
  const raw = files.read('package.json')
  if (raw === null) return null
  let pkg: z.infer<typeof packageJsonSchema> = {}
  try {
    pkg = packageJsonSchema.parse(JSON.parse(raw))
  } catch {
    // Bozuk package.json: yine Node projesidir, komutlar bilinmez.
  }
  const fromField = pkg.packageManager?.split('@')[0]
  const pm =
    fromField && ['npm', 'pnpm', 'yarn', 'bun'].includes(fromField)
      ? fromField
      : files.names.has('pnpm-lock.yaml')
        ? 'pnpm'
        : files.names.has('yarn.lock')
          ? 'yarn'
          : files.names.has('bun.lockb') || files.names.has('bun.lock')
            ? 'bun'
            : 'npm'
  const scripts = pkg.scripts ?? {}
  const run = (name: string) => (pm === 'npm' ? (name === 'test' ? 'npm test' : `npm run ${name}`) : pm === 'yarn' ? `yarn ${name}` : `${pm} run ${name}`)
  const has = (name: string) => {
    const script = scripts[name]
    // npm init'in varsayılan "no test specified" betiği gerçek bir test değildir.
    return script !== undefined && !/no test specified/.test(script)
  }
  const deps = { ...pkg.dependencies, ...pkg.devDependencies }
  const typescript = files.names.has('tsconfig.json') || 'typescript' in deps
  const typecheckScript = ['typecheck', 'type-check', 'tsc', 'check-types'].find(has)
  return {
    kinds: [typescript ? 'Node.js (TypeScript)' : 'Node.js (JavaScript)'],
    packageManager: pm,
    typecheck: typecheckScript ? run(typecheckScript) : files.names.has('tsconfig.json') ? (pm === 'npm' ? 'npx tsc --noEmit' : `${pm} exec tsc --noEmit`) : null,
    test: has('test') ? run('test') : null,
    build: has('build') ? run('build') : null,
    lint: has('lint') ? run('lint') : null,
  }
}

function pythonProject(files: ProjectFiles): ProjectInfo | null {
  const markers = ['pyproject.toml', 'requirements.txt', 'setup.py', 'setup.cfg', 'Pipfile']
  if (!markers.some((m) => files.names.has(m))) return null
  const pyproject = files.read('pyproject.toml') ?? ''
  const pm = files.names.has('uv.lock')
    ? 'uv'
    : files.names.has('poetry.lock') || /\[tool\.poetry\]/.test(pyproject)
      ? 'poetry'
      : files.names.has('Pipfile')
        ? 'pipenv'
        : 'pip'
  const prefix = pm === 'uv' ? 'uv run ' : pm === 'poetry' ? 'poetry run ' : pm === 'pipenv' ? 'pipenv run ' : ''
  const configured = (tool: string) => new RegExp(`\\[tool\\.${tool}\\b|\\b${tool}\\b`).test(pyproject) || files.names.has(`${tool}.ini`) || files.names.has(`.${tool}.toml`)
  const pytest = configured('pytest') || files.names.has('pytest.ini') || files.names.has('tests') || files.names.has('conftest.py')
  return {
    kinds: ['Python'],
    packageManager: pm,
    typecheck: configured('mypy') ? `${prefix}mypy .` : configured('pyright') ? `${prefix}pyright` : null,
    test: pytest ? `${prefix}pytest` : null,
    build: files.names.has('pyproject.toml') ? (pm === 'poetry' ? 'poetry build' : pm === 'uv' ? 'uv build' : 'python -m build') : null,
    lint: configured('ruff') ? `${prefix}ruff check .` : configured('flake8') ? `${prefix}flake8` : null,
  }
}

function simple(kind: string, pm: string, commands: ProjectCommands): ProjectInfo {
  return { kinds: [kind], packageManager: pm, ...commands }
}

/** Saf algılama: verilen dosya adları/içerikleriyle proje türlerini ve doğrulama komutlarını belirler. */
export function detectProject(files: ProjectFiles): ProjectInfo | null {
  const found: ProjectInfo[] = []
  const node = nodeProject(files)
  if (node) found.push(node)
  if (files.names.has('deno.json') || files.names.has('deno.jsonc')) {
    found.push(simple('Deno', 'deno', { typecheck: 'deno check .', test: 'deno test', build: null, lint: 'deno lint' }))
  }
  const python = pythonProject(files)
  if (python) found.push(python)
  const names = [...files.names]
  if (names.some((n) => /\.(sln|slnx|csproj|fsproj|vbproj)$/i.test(n)) || files.names.has('global.json')) {
    found.push(simple('.NET', 'dotnet', { typecheck: 'dotnet build', test: 'dotnet test', build: 'dotnet build', lint: 'dotnet format --verify-no-changes' }))
  }
  if (files.names.has('Cargo.toml')) {
    found.push(simple('Rust', 'cargo', { typecheck: 'cargo check', test: 'cargo test', build: 'cargo build', lint: 'cargo clippy' }))
  }
  if (files.names.has('go.mod')) {
    found.push(simple('Go', 'go', { typecheck: 'go vet ./...', test: 'go test ./...', build: 'go build ./...', lint: 'go vet ./...' }))
  }
  if (files.names.has('pom.xml')) {
    const mvn = files.names.has('mvnw') ? './mvnw' : 'mvn'
    found.push(simple('Java (Maven)', 'maven', { typecheck: `${mvn} -q compile`, test: `${mvn} test`, build: `${mvn} package`, lint: null }))
  } else if (files.names.has('build.gradle') || files.names.has('build.gradle.kts')) {
    const gradle = files.names.has('gradlew') ? './gradlew' : 'gradle'
    found.push(simple('Java/Kotlin (Gradle)', 'gradle', { typecheck: `${gradle} compileJava`, test: `${gradle} test`, build: `${gradle} build`, lint: null }))
  }
  if (files.names.has('composer.json')) {
    found.push(simple('PHP', 'composer', { typecheck: null, test: files.names.has('phpunit.xml') || files.names.has('phpunit.xml.dist') ? 'vendor/bin/phpunit' : null, build: null, lint: null }))
  }
  if (files.names.has('Gemfile')) {
    found.push(simple('Ruby', 'bundler', { typecheck: null, test: files.names.has('spec') ? 'bundle exec rspec' : files.names.has('Rakefile') ? 'bundle exec rake test' : null, build: null, lint: null }))
  }
  const [first, ...rest] = found
  if (!first) return null
  // Çok dilli depolarda ilk algılanan birincil sayılır; komutları eksikse diğerlerinden tamamlanır.
  const pick = (key: keyof ProjectCommands) => first[key] ?? rest.find((p) => p[key])?.[key] ?? null
  return {
    kinds: found.flatMap((p) => p.kinds),
    packageManager: first.packageManager,
    typecheck: pick('typecheck'),
    test: pick('test'),
    build: pick('build'),
    lint: pick('lint'),
  }
}

const READ_FILES = ['package.json', 'pyproject.toml']
const MAX_READ_BYTES = 200_000

/** Kök dizini okuyup detectProject'e verir; yalnızca kökteki adlar ve iki küçük dosya okunur. */
export async function detectProjectAt(root: string): Promise<ProjectInfo | null> {
  let entries: string[]
  try {
    entries = await readdir(root)
  } catch {
    return null
  }
  const contents = new Map<string, string>()
  for (const name of READ_FILES.filter((n) => entries.includes(n))) {
    try {
      const text = await readFile(path.join(root, name), 'utf8')
      if (text.length <= MAX_READ_BYTES) contents.set(name, text)
    } catch {
      // Okunamayan dosya algılamayı engellemez.
    }
  }
  return detectProject({ names: new Set(entries), read: (n) => contents.get(n) ?? null })
}

export function formatProjectSection(info: ProjectInfo | null): string | null {
  if (!info) return null
  const lines = [`# PROJECT TYPE`, `Detected: ${info.kinds.join(', ')}${info.packageManager ? `; package manager: ${info.packageManager}` : ''}`]
  for (const key of ['typecheck', 'test', 'build', 'lint'] as const) if (info[key]) lines.push(`${key}: ${info[key]}`)
  lines.push('Use these commands to verify changes (run them from the project root).')
  return lines.join('\n')
}

/** autoVerify sırası: önce tip denetimi, sonra test. Ayrı komutlar: PowerShell 5.1'de && yok. */
export function verifyCommands(info: ProjectInfo | null): string[] {
  if (!info) return []
  return [...new Set([info.typecheck, info.test].filter((c): c is string => c !== null))]
}
