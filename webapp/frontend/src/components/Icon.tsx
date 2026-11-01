/**
 * Иконки — один SVG-спрайт public/icons.svg (как в ozonware): 24px viewBox, обводка 1.75,
 * скруглённые концы. Новая иконка = новый <symbol> в спрайте, не эмодзи и не текстовый глиф.
 */
export type IconName =
  | 'sun'
  | 'moon'
  | 'menu'
  | 'sidebar'
  | 'chevron-down'
  | 'upload'
  | 'x'
  | 'check'
  | 'alert'
  | 'download'
  | 'arrow-right'
  | 'eye'
  | 'eye-off'
  | 'list'
  | 'cube'
  | 'calendar'
  | 'shield'
  | 'logout'
  | 'user'
  | 'file'
  | 'gauge'
  | 'copy'
  | 'stop'
  | 'layers'
  | 'refresh'
  | 'plus'
  | 'info'
  | 'focus';

export default function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg className="ic" width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <use href={`/icons.svg#${name}`} />
    </svg>
  );
}
