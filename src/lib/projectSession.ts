/** Renderer-local generation of project activation attempts; reads never advance it. */
let epoch = 0
export const projectSessionEpoch = () => epoch
export const advanceProjectSession = () => ++epoch
export const isProjectSession = (captured: number) => captured === epoch
export function assertProjectSession(captured: number) {
  if (!isProjectSession(captured))
    throw new Error('The project session changed. Your draft is kept; reload before saving again.')
}
