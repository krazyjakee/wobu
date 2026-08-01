import { useEffect, useRef } from 'react'
import * as THREE from 'three'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'

export default function MeshViewport({
  url,
  turntable,
  wireframe,
  onError,
}: {
  url: string
  turntable: boolean
  wireframe: boolean
  onError: (message: string) => void
}) {
  const host = useRef<HTMLDivElement>(null)
  const object = useRef<THREE.Object3D | null>(null)
  const turntableRef = useRef(turntable)
  const wireframeRef = useRef(wireframe)

  useEffect(() => {
    turntableRef.current = turntable
  }, [turntable])

  useEffect(() => {
    wireframeRef.current = wireframe
    setWireframe(object.current, wireframe)
  }, [wireframe])

  useEffect(() => {
    const element = host.current
    if (!element) return
    let disposed = false
    let frame = 0

    const scene = new THREE.Scene()
    scene.background = new THREE.Color(0x111319)
    const camera = new THREE.PerspectiveCamera(40, 1, 0.01, 10_000)
    const renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'high-performance' })
    renderer.outputColorSpace = THREE.SRGBColorSpace
    renderer.toneMapping = THREE.ACESFilmicToneMapping
    renderer.toneMappingExposure = 1.1
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
    element.appendChild(renderer.domElement)

    const controls = new OrbitControls(camera, renderer.domElement)
    controls.enableDamping = true
    controls.screenSpacePanning = true
    controls.minDistance = 0.05
    controls.maxDistance = 5_000

    scene.add(new THREE.HemisphereLight(0xdde8ff, 0x2f241e, 2.2))
    const key = new THREE.DirectionalLight(0xffffff, 3.2)
    key.position.set(3, 5, 4)
    scene.add(key)
    const rim = new THREE.DirectionalLight(0x88aaff, 1.5)
    rim.position.set(-4, 2, -3)
    scene.add(rim)

    const turntableRoot = new THREE.Group()
    scene.add(turntableRoot)
    const grid = new THREE.GridHelper(10, 10, 0x526070, 0x2a3039)
    scene.add(grid)
    const marker = scaleMarker()
    scene.add(marker)

    const resize = () => {
      const width = Math.max(element.clientWidth, 1)
      const height = Math.max(element.clientHeight, 1)
      renderer.setSize(width, height, false)
      camera.aspect = width / height
      camera.updateProjectionMatrix()
    }
    const observer = new ResizeObserver(resize)
    observer.observe(element)
    resize()

    const clock = new THREE.Clock()
    const draw = () => {
      frame = requestAnimationFrame(draw)
      if (turntableRef.current) turntableRoot.rotation.y += clock.getDelta() * 0.45
      else clock.getDelta()
      controls.update()
      renderer.render(scene, camera)
    }
    draw()

    new GLTFLoader().load(
      url,
      (gltf) => {
        if (disposed) {
          disposeObject(gltf.scene)
          return
        }
        object.current = gltf.scene
        setWireframe(gltf.scene, wireframeRef.current)
        const box = new THREE.Box3().setFromObject(gltf.scene)
        if (box.isEmpty()) {
          onError('The GLB contains no visible geometry.')
          disposeObject(gltf.scene)
          object.current = null
          return
        }
        const size = box.getSize(new THREE.Vector3())
        const centre = box.getCenter(new THREE.Vector3())
        gltf.scene.position.set(-centre.x, -box.min.y, -centre.z)
        turntableRoot.add(gltf.scene)

        const span = Math.max(size.x, size.y, size.z, 1)
        const distance = span / (2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)))
        controls.target.set(0, size.y / 2, 0)
        camera.near = Math.max(span / 10_000, 0.001)
        camera.far = Math.max(span * 100, 100)
        camera.position.set(distance * 0.9, size.y * 0.65, distance * 1.25)
        camera.updateProjectionMatrix()
        controls.update()
      },
      undefined,
      (error) => {
        if (!disposed) {
          onError(error instanceof Error ? error.message : 'The GLB could not be loaded.')
        }
      },
    )

    return () => {
      disposed = true
      cancelAnimationFrame(frame)
      observer.disconnect()
      controls.dispose()
      if (object.current) disposeObject(object.current)
      object.current = null
      grid.geometry.dispose()
      disposeMaterials(grid.material)
      marker.geometry.dispose()
      disposeMaterials(marker.material)
      renderer.renderLists.dispose()
      renderer.dispose()
      renderer.forceContextLoss()
      renderer.domElement.remove()
    }
  }, [onError, url])

  return <div className="mesh-canvas" ref={host} aria-label="Interactive 3D mesh viewer" />
}

function scaleMarker(): THREE.LineSegments {
  const points = [
    new THREE.Vector3(-0.5, 0, 0),
    new THREE.Vector3(-0.5, 1, 0),
    new THREE.Vector3(-0.56, 0, 0),
    new THREE.Vector3(-0.44, 0, 0),
    new THREE.Vector3(-0.56, 1, 0),
    new THREE.Vector3(-0.44, 1, 0),
  ]
  return new THREE.LineSegments(
    new THREE.BufferGeometry().setFromPoints(points),
    new THREE.LineBasicMaterial({ color: 0xff8866 }),
  )
}

function setWireframe(root: THREE.Object3D | null, enabled: boolean) {
  root?.traverse((child) => {
    if (!(child instanceof THREE.Mesh)) return
    const materials = Array.isArray(child.material) ? child.material : [child.material]
    for (const material of materials) {
      if ('wireframe' in material) {
        ;(material as THREE.MeshBasicMaterial).wireframe = enabled
        material.needsUpdate = true
      }
    }
  })
}

function disposeObject(root: THREE.Object3D) {
  root.traverse((child) => {
    if (!(child instanceof THREE.Mesh)) return
    child.geometry.dispose()
    const materials = Array.isArray(child.material) ? child.material : [child.material]
    disposeMaterials(materials)
  })
}

function disposeMaterial(material: THREE.Material) {
  for (const value of Object.values(material)) {
    if (value instanceof THREE.Texture) value.dispose()
  }
  material.dispose()
}

function disposeMaterials(materials: THREE.Material | THREE.Material[]) {
  for (const material of Array.isArray(materials) ? materials : [materials]) {
    disposeMaterial(material)
  }
}
