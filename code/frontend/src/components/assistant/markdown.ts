/** Block structure of the assistant's markdown subset (see MarkdownLite). */

const BULLET = /^\s*[-*•]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;

export type Block =
  | { kind: 'p'; lines: string[] }
  | { kind: 'ul' | 'ol'; items: string[] }
  | { kind: 'code'; text: string }
  | { kind: 'h'; text: string };

export function parseBlocks(text: string): Block[] {
  const blocks: Block[] = [];
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim().startsWith('```')) {
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].trim().startsWith('```')) {
        body.push(lines[i]);
        i += 1;
      }
      i += 1; // closing fence (or end of a still-streaming block)
      blocks.push({ kind: 'code', text: body.join('\n') });
      continue;
    }
    if (!line.trim()) {
      i += 1;
      continue;
    }
    const heading = /^#{1,6}\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({ kind: 'h', text: heading[1] });
      i += 1;
      continue;
    }
    const listKind = BULLET.test(line) ? 'ul' : NUMBERED.test(line) ? 'ol' : null;
    if (listKind) {
      const re = listKind === 'ul' ? BULLET : NUMBERED;
      const items: string[] = [];
      let match: RegExpExecArray | null;
      while (i < lines.length && (match = re.exec(lines[i]))) {
        items.push(match[1]);
        i += 1;
      }
      blocks.push({ kind: listKind, items });
      continue;
    }
    const paragraph: string[] = [];
    while (i < lines.length && lines[i].trim() && !lines[i].trim().startsWith('```') && !BULLET.test(lines[i]) && !NUMBERED.test(lines[i]) && !/^#{1,6}\s/.test(lines[i])) {
      paragraph.push(lines[i]);
      i += 1;
    }
    blocks.push({ kind: 'p', lines: paragraph });
  }
  return blocks;
}
