import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/react';
import { MarkdownLite } from './MarkdownLite';
import { parseBlocks } from './markdown';
import { SseReader } from '../../api/assistant';

describe('MarkdownLite', () => {
  it('renders lists, code, emphasis and never HTML', () => {
    const { container } = render(
      <MarkdownLite text={'Status:\n- **jellyfin** is `1.2.3`\n- sonarr\n\n1. Open Apps\n2. Update\n\n```\nkubectl get pods\n```\n<img src=x onerror=alert(1)>'} />,
    );
    expect(container.querySelectorAll('ul li')).toHaveLength(2);
    expect(container.querySelectorAll('ol li')).toHaveLength(2);
    expect(container.querySelector('strong')?.textContent).toBe('jellyfin');
    expect(container.querySelector('li code')?.textContent).toBe('1.2.3');
    expect(container.querySelector('pre')?.textContent).toBe('kubectl get pods');
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toContain('<img src=x onerror=alert(1)>');
  });

  it('keeps an unterminated code fence while streaming', () => {
    expect(parseBlocks('```\npartial')).toEqual([{ kind: 'code', text: 'partial' }]);
  });
});

describe('SseReader', () => {
  it('parses events split across chunks and skips unknown ones', () => {
    const reader = new SseReader();
    expect(reader.push('event: text_delta\ndata: {"te')).toEqual([]);
    expect(reader.push('xt":"hi"}\n\nevent: ping\ndata: {}\n\nevent: done\r\ndata: {"message_id":3}\r\n\r\n')).toEqual([
      { event: 'text_delta', text: 'hi' },
      { event: 'done', message_id: 3 },
    ]);
  });
});
