import { Icon } from '../Icon'

/** Every unbuilt tab says which milestone brings it. No placeholder data. */
export function TabEmpty({
  icon,
  title,
  milestone,
  body,
}: {
  icon: string
  title: string
  milestone: string
  body: string
}) {
  return (
    <div className="empty">
      <Icon name={icon} size="xl" />
      <h3>{title}</h3>
      <span className="milestone">{milestone}</span>
      <p>{body}</p>
    </div>
  )
}
