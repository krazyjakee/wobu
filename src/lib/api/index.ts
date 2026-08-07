/**
 * Typed wrappers over the Rust command surface.
 *
 * Tauri v2 converts camelCase JS argument keys to snake_case Rust parameters,
 * and every payload struct is serde(rename_all = "camelCase"), so the shapes
 * below are exactly what crosses the bridge.
 *
 * One module per command group, mirroring `src-tauri/src/commands/`. This
 * barrel is the whole surface: importing from `lib/api` gets everything, and
 * nothing outside this directory should reach into a module by name.
 */
export * from './assets'
export * from './call'
export * from './collab'
export * from './diagnostics'
export * from './enhance'
export * from './generate'
export * from './generations'
export * from './influence'
export * from './jobs'
export * from './model'
export * from './nodes'
export * from './project'
export * from './providers'
export * from './sync'
