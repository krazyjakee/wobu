import type { Capability } from '../../lib/api'

/**
 * One credential a provider needs before it will answer.
 *
 * A list rather than a string because Tencent's is a SecretId/SecretKey *pair*
 * signed together, not a bearer token — `keys.rs` registers it as two credential
 * entries for that reason, and a pane that assumed one field per provider would
 * have nowhere to put the second.
 */
export interface Credential {
  /** The `wobu/<id>` credential entry, and the id every command here takes. */
  id: string
  label: string
}

export interface ProviderDef {
  /** The id `project.json` carries. */
  id: string
  label: string
  /** Empty for a backend that authenticates nothing. See ComfyUI. */
  credentials: Credential[]
  /** Where the user goes to get one. */
  where?: string
  /** Whether this build has an adapter that can check a key for it. */
  checkable?: boolean
  /** Said instead of a key field, for a provider that needs none. */
  instead?: string
  /** Material capability or licence difference a user must see before selecting it. */
  tierNote?: string
  /** Whether credentials need the Tencent account-safety setup shown before the fields. */
  hunyuanOnboarding?: boolean
}

export const ANTHROPIC: ProviderDef = {
  id: 'anthropic',
  label: 'Anthropic',
  credentials: [{ id: 'anthropic', label: 'API key' }],
  where: 'the Claude Console',
  checkable: true,
}

export const GEMINI: ProviderDef = {
  id: 'gemini',
  label: 'Gemini',
  credentials: [{ id: 'gemini', label: 'API key' }],
  where: 'Google AI Studio',
  checkable: true,
}

export const COMFYUI: ProviderDef = {
  id: 'comfyui',
  label: 'ComfyUI',
  credentials: [],
  instead:
    'ComfyUI needs no key — it is a server you run yourself. Its machine-local address is ' +
    'configured and checked below instead of being written into the shared project.',
}

export const HUNYUAN3D: ProviderDef = {
  id: 'hunyuan3d',
  label: 'Tencent Hunyuan3D',
  credentials: [
    { id: 'tencent-secret-id', label: 'Secret ID' },
    { id: 'tencent-secret-key', label: 'Secret key' },
  ],
  where: 'a Tencent CAM sub-account',
  hunyuanOnboarding: true,
}

export const HUNYUAN_REGIONS = [
  { id: 'ap-singapore', label: 'Singapore (Asia-Pacific)' },
  { id: 'na-siliconvalley', label: 'Silicon Valley (North America)' },
  { id: 'eu-frankfurt', label: 'Frankfurt (Europe)' },
] as const

export const COMFYUI_MESH: ProviderDef = {
  id: 'comfyui',
  label: 'Local 2.1 (ComfyUI)',
  credentials: [],
  tierNote:
    'Explicit local tier — never an automatic fallback when a Tencent key is absent. It uses the ' +
    'older, lower-quality Hunyuan3D 2.1 shape model: one front image, geometry-only output and at ' +
    'least 10 GB VRAM, with no per-job fee and image data staying on your ComfyUI machine. Wobu ' +
    'does not install its weights or nodes. Tencent’s model licence excludes the EU, UK and South ' +
    'Korea, so check that you are permitted to use it where you are.',
}

export interface CapabilityDef {
  capability: Capability
  label: string
  /** What in the app uses this choice. */
  used: string
  icon: string
  providers: ProviderDef[]
  /** Whether choosing a model here means anything to this build. */
  model: boolean
  /**
   * What runs when `project.json` names nothing.
   *
   * Not a display default: `enhance.rs` really does fall back to Anthropic, so
   * a pane that showed "nothing chosen" and left it there would be describing a
   * project that spends money at a provider it never mentioned. Every world
   * made before this pane existed is in exactly that state.
   */
  fallback?: ProviderDef
  /** What the current product surface does with this capability. */
  activeNote?: string
}

/**
 * Three capabilities, chosen separately.
 *
 * Enhancing with Gemini, generating on a ComfyUI running under your desk and
 * meshing through Hunyuan3D is the ordinary combination rather than the exotic
 * one, and a single "provider" setting could not express it at all.
 */
export const CAPABILITIES: CapabilityDef[] = [
  {
    capability: 'text',
    label: 'Text',
    used: 'Enhance',
    icon: 'spark',
    providers: [ANTHROPIC, GEMINI],
    model: true,
    fallback: ANTHROPIC,
  },
  {
    capability: 'image',
    label: 'Image',
    used: 'Generate',
    icon: 'image',
    providers: [COMFYUI, GEMINI],
    model: false,
    activeNote:
      'Generate and Forge use this choice for entity images, variant grids and scene compositions.',
  },
  {
    capability: 'mesh',
    label: 'Mesh',
    used: 'Concept 3D',
    icon: 'cube',
    providers: [HUNYUAN3D, COMFYUI_MESH],
    model: false,
    activeNote:
      'Concept 3D reconstructs a mesh from a reviewed turnaround, and views and exports completed GLBs from the project asset library.',
  },
]

/**
 * Every provider that has a key, once each.
 *
 * Once, because a key is not per capability: Gemini writes text and makes
 * pictures on the same credential, and listing it twice would ask the user for
 * two keys and store one. This ordering is the order the key rows appear in.
 */
export const KEYED: ProviderDef[] = [ANTHROPIC, GEMINI, HUNYUAN3D, COMFYUI]

/**
 * Module-level so the array identity is stable — it is part of the React Query
 * key, and a fresh array every render would refetch on every render.
 */
export const CREDENTIAL_IDS: string[] = KEYED.flatMap((p) => p.credentials.map((c) => c.id))
