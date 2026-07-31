import type { CSSProperties } from 'react'

export type IconSize = 'sm' | 'md' | 'xl'

const CLS: Record<IconSize, string> = { sm: 'ic ic-sm', md: 'ic', xl: 'ic ic-xl' }

export function Icon({
  name,
  size = 'md',
  style,
  className,
}: {
  /** sprite id without the `i-` prefix, e.g. `species` */
  name: string
  size?: IconSize
  style?: CSSProperties
  className?: string
}) {
  return (
    <svg className={className ? `${CLS[size]} ${className}` : CLS[size]} style={style} aria-hidden>
      <use href={`#i-${name}`} />
    </svg>
  )
}
