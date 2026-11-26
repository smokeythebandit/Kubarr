import { Fragment, type ReactNode } from 'react';
import { parseBlocks } from './markdown';

/**
 * Renders the small markdown subset the assistant is asked to use:
 * paragraphs, `-`/`*`/`1.` lists, ```code blocks```, `inline code`,
 * **bold**, _italic_ / *italic* and ### headings. Everything is built as
 * React elements, so no HTML in the text is ever interpreted.
 */

const INLINE = /(`[^`\n]+`|\*\*[^*\n]+\*\*|__[^_\n]+__|\*[^*\n]+\*|_[^_\n]+_)/g;

function renderInline(text: string): ReactNode[] {
  return text.split(INLINE).map((part, i) => {
    if (!part) return null;
    if (part.length > 2 && part.startsWith('`') && part.endsWith('`')) {
      return (
        <code key={i} className="rounded bg-gray-100 dark:bg-gray-700/70 px-1 py-0.5 font-mono text-[0.85em]">
          {part.slice(1, -1)}
        </code>
      );
    }
    if (part.length > 4 && ((part.startsWith('**') && part.endsWith('**')) || (part.startsWith('__') && part.endsWith('__')))) {
      return <strong key={i} className="font-semibold">{part.slice(2, -2)}</strong>;
    }
    if (part.length > 2 && ((part.startsWith('*') && part.endsWith('*')) || (part.startsWith('_') && part.endsWith('_')))) {
      return <em key={i}>{part.slice(1, -1)}</em>;
    }
    return <Fragment key={i}>{part}</Fragment>;
  });
}

export function MarkdownLite({ text }: { text: string }) {
  return (
    <div className="space-y-2 break-words">
      {parseBlocks(text).map((block, i) => {
        switch (block.kind) {
          case 'code':
            return (
              <pre key={i} className="overflow-x-auto rounded-md bg-gray-100 dark:bg-gray-900/70 p-2 font-mono text-xs leading-relaxed">
                <code>{block.text}</code>
              </pre>
            );
          case 'h':
            return <p key={i} className="font-semibold">{renderInline(block.text)}</p>;
          case 'ul':
            return (
              <ul key={i} className="list-disc space-y-0.5 pl-5">
                {block.items.map((item, j) => <li key={j}>{renderInline(item)}</li>)}
              </ul>
            );
          case 'ol':
            return (
              <ol key={i} className="list-decimal space-y-0.5 pl-5">
                {block.items.map((item, j) => <li key={j}>{renderInline(item)}</li>)}
              </ol>
            );
          default:
            return (
              <p key={i}>
                {block.lines.map((line, j) => (
                  <Fragment key={j}>
                    {j > 0 && <br />}
                    {renderInline(line)}
                  </Fragment>
                ))}
              </p>
            );
        }
      })}
    </div>
  );
}
