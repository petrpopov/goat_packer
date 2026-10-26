import { useMemo } from 'react';
import { parseMarkdown } from '@/utils/markdown';
import type { Inline } from '@/utils/markdown';

function InlineText({ parts }: { parts: Inline[] }) {
  return (
    <>
      {parts.map((part, index) =>
        part.kind === 'bold' ? (
          <strong key={index}>{part.text}</strong>
        ) : part.kind === 'code' ? (
          <code key={index} className="mono">
            {part.text}
          </code>
        ) : (
          <span key={index}>{part.text}</span>
        )
      )}
    </>
  );
}

/** Безопасный показ Markdown: только React-элементы из разобранного текста, без innerHTML. */
export default function MarkdownView({ source }: { source: string }) {
  const blocks = useMemo(() => parseMarkdown(source), [source]);
  return (
    <div className="markdown">
      {blocks.map((block, index) => {
        switch (block.kind) {
          case 'heading': {
            const Tag = block.level === 1 ? 'h3' : 'h4';
            return (
              <Tag key={index}>
                <InlineText parts={block.inline} />
              </Tag>
            );
          }
          case 'paragraph':
            return (
              <p key={index}>
                <InlineText parts={block.inline} />
              </p>
            );
          case 'list': {
            const items = block.items.map((item, itemIndex) => (
              <li key={itemIndex}>
                <InlineText parts={item} />
              </li>
            ));
            return block.ordered ? <ol key={index}>{items}</ol> : <ul key={index}>{items}</ul>;
          }
          case 'code':
            return (
              <pre key={index} className="markdown-code">
                {block.text}
              </pre>
            );
        }
      })}
    </div>
  );
}
