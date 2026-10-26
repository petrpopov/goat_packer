/**
 * Минимальный разбор Markdown инструкции калибровки (instructions.md от ams_calibration):
 * заголовки, абзацы, списки, блоки кода, **жирный** и `код`. Результат — данные, которые
 * компонент превращает в React-элементы; сырой HTML никогда не вставляется, поэтому текст
 * файла не может исполниться на странице.
 */

export type Inline = { kind: 'text' | 'bold' | 'code'; text: string };

export type MarkdownBlock =
  | { kind: 'heading'; level: 1 | 2 | 3; inline: Inline[] }
  | { kind: 'paragraph'; inline: Inline[] }
  | { kind: 'list'; ordered: boolean; items: Inline[][] }
  | { kind: 'code'; text: string };

/** `код` и **жирный**; непарные маркеры остаются обычным текстом. */
export function parseInline(text: string): Inline[] {
  const result: Inline[] = [];
  const pattern = /`([^`]+)`|\*\*([^*]+)\*\*/g;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const at = match.index ?? 0;
    if (at > last) result.push({ kind: 'text', text: text.slice(last, at) });
    result.push(match[1] !== undefined ? { kind: 'code', text: match[1] } : { kind: 'bold', text: match[2] });
    last = at + match[0].length;
  }
  if (last < text.length) result.push({ kind: 'text', text: text.slice(last) });
  return result;
}

const HEADING = /^(#{1,3})\s+(.*)$/;
const BULLET = /^[-*]\s+(.*)$/;
const NUMBERED = /^\d+[.)]\s+(.*)$/;

export function parseMarkdown(source: string): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  const lines = source.replace(/\r\n?/g, '\n').split('\n');
  let paragraph: string[] = [];
  let list: { ordered: boolean; items: string[] } | null = null;

  const flush = () => {
    if (paragraph.length) blocks.push({ kind: 'paragraph', inline: parseInline(paragraph.join(' ')) });
    if (list) blocks.push({ kind: 'list', ordered: list.ordered, items: list.items.map(parseInline) });
    paragraph = [];
    list = null;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (trimmed.startsWith('```')) {
      flush();
      const code: string[] = [];
      for (i++; i < lines.length && !lines[i].trim().startsWith('```'); i++) code.push(lines[i]);
      blocks.push({ kind: 'code', text: code.join('\n') });
      continue;
    }
    if (trimmed === '') {
      flush();
      continue;
    }
    const heading = trimmed.match(HEADING);
    if (heading) {
      flush();
      blocks.push({ kind: 'heading', level: heading[1].length as 1 | 2 | 3, inline: parseInline(heading[2]) });
      continue;
    }
    const bullet = trimmed.match(BULLET);
    const numbered = bullet ? null : trimmed.match(NUMBERED);
    const item = bullet ?? numbered;
    if (item) {
      const ordered = numbered !== null;
      if (paragraph.length || (list && list.ordered !== ordered)) flush();
      list ??= { ordered, items: [] };
      list.items.push(item[1]);
      continue;
    }
    // Продолжение пункта списка или абзаца (строки, перенесённые в исходнике).
    if (list) list.items[list.items.length - 1] += ` ${trimmed}`;
    else paragraph.push(trimmed);
  }
  flush();
  return blocks;
}
