// [g21] Tembel three.js paketi (#182): GLB görüntüleyici. Model gltf-pack.ts ile önceden tek GLB'ye ve data:
//      dokulara çevrilir; burada hiçbir dış URL yüklenmez (CSP ayrıca engeller). Draco/meshopt/KTX2 gibi harici
//      çözücü (wasm) gerektiren uzantılar yüklenmez: GLTFLoader bunları hata olarak bildirir.
import { Box3, DirectionalLight, HemisphereLight, LoadingManager, Mesh, PerspectiveCamera, Scene, TextureLoader, Vector3, WebGLRenderer, type Material, type Texture } from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'

export interface ModelStats {
  meshes: number
  triangles: number
  materials: number
  animations: number
}

export interface ModelHandle {
  stats: ModelStats
  resetView(): void
  setAutoRotate(on: boolean): void
  dispose(): void
}

async function mountModel(canvas: HTMLCanvasElement, glb: ArrayBuffer): Promise<ModelHandle> {
  const renderer = new WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: 'low-power' })
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
  const scene = new Scene()
  scene.add(new HemisphereLight(0xffffff, 0x444444, 2.2))
  const key = new DirectionalLight(0xffffff, 2)
  key.position.set(3, 5, 4)
  scene.add(key)
  const camera = new PerspectiveCamera(45, 1, 0.01, 1000)
  const controls = new OrbitControls(camera, canvas)
  controls.enableDamping = true

  const manager = new LoadingManager()
  // Dokular data: URI'dir; ImageBitmapLoader fetch kullanır (CSP connect-src 'none' engeller), <img> tabanlı
  // TextureLoader ise img-src data: ile çalışır.
  manager.addHandler(/^data:image\//, new TextureLoader(manager))
  const dispose = () => {
    renderer.setAnimationLoop(null)
    resize.disconnect()
    controls.dispose()
    scene.traverse((obj) => {
      if (!(obj instanceof Mesh)) return
      obj.geometry.dispose()
      const materials: Material[] = Array.isArray(obj.material) ? obj.material : [obj.material]
      for (const m of materials) {
        for (const value of Object.values(m)) if (value && typeof value === 'object' && 'isTexture' in value) (value as Texture).dispose()
        m.dispose()
      }
    })
    renderer.dispose()
    renderer.forceContextLoss()
  }
  const resize = new ResizeObserver(() => {
    const { clientWidth: w, clientHeight: h } = canvas
    if (w === 0 || h === 0) return
    renderer.setSize(w, h, false)
    camera.aspect = w / h
    camera.updateProjectionMatrix()
  })
  resize.observe(canvas)

  let gltf
  try {
    gltf = await new GLTFLoader(manager).parseAsync(glb, '')
  } catch (err) {
    dispose()
    throw err
  }
  scene.add(gltf.scene)

  const box = new Box3().setFromObject(gltf.scene)
  const size = box.getSize(new Vector3())
  const center = box.getCenter(new Vector3())
  const radius = Math.max(size.length() / 2, 1e-3)
  const resetView = () => {
    const distance = radius / Math.sin((camera.fov * Math.PI) / 360)
    camera.near = distance / 100
    camera.far = distance * 100
    camera.position.copy(center).add(new Vector3(0.6, 0.4, 1).normalize().multiplyScalar(distance))
    camera.updateProjectionMatrix()
    controls.target.copy(center)
    controls.update()
  }
  resetView()

  const stats: ModelStats = { meshes: 0, triangles: 0, materials: 0, animations: gltf.animations.length }
  const materialSet = new Set<Material>()
  gltf.scene.traverse((obj) => {
    if (!(obj instanceof Mesh)) return
    stats.meshes++
    const g = obj.geometry
    stats.triangles += Math.floor((g.index ? g.index.count : (g.getAttribute('position')?.count ?? 0)) / 3)
    for (const m of Array.isArray(obj.material) ? obj.material : [obj.material]) materialSet.add(m)
  })
  stats.materials = materialSet.size

  renderer.setAnimationLoop(() => {
    controls.update()
    renderer.render(scene, camera)
  })
  return {
    stats,
    resetView,
    setAutoRotate: (on) => {
      controls.autoRotate = on
    },
    dispose,
  }
}

const api = { mountModel }
export type G21ThreeApi = typeof api
;(window as unknown as { __g21Three: G21ThreeApi }).__g21Three = api
