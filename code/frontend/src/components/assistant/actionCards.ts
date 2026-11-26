import type { ActionCard, ChatMessage } from '../../api/assistant';
import type { ShownMessage } from './AssistantChat';

/** "4:59" until the action expires. */
export function formatRemaining(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

/** Messages with each action card attached to the answer that proposed it. */
export function withActions(messages: ChatMessage[], actions: ActionCard[] = []): ShownMessage[] {
  const shown: ShownMessage[] = messages.map((m) => ({ ...m }));
  const lastAssistant = [...shown].reverse().find((m) => m.role === 'assistant');
  for (const card of actions) {
    const owner = shown.find((m) => m.id === card.message_id && m.role === 'assistant') ?? lastAssistant;
    if (owner) owner.actions = [...(owner.actions ?? []), card];
  }
  return shown;
}
