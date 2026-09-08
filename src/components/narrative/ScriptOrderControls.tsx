export function ScriptOrderControls<T>({
  label,
  items,
  index,
  onChange,
}: {
  label: string
  items: T[]
  index: number
  onChange: (items: T[]) => void
}) {
  const move = (by: number) => {
    const next = [...items]
    const to = index + by
    if (to < 0 || to >= items.length) return
    ;[next[index], next[to]] = [next[to]!, next[index]!]
    onChange(next)
  }
  return (
    <div className="nrt-script-actions">
      <button className="btn" disabled={index === 0} onClick={() => move(-1)}>
        Move {label} up
      </button>
      <button className="btn" disabled={index === items.length - 1} onClick={() => move(1)}>
        Move {label} down
      </button>
    </div>
  )
}
