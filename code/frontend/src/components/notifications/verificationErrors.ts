// Helpers for the destination verification UI: axios error parsing and code input.

type ApiError = {
  response?: { status?: number; data?: { detail?: string }; headers?: Record<string, string | undefined> };
};

export function errorDetail(err: unknown, fallback: string): string {
  return (err as ApiError).response?.data?.detail || fallback;
}

/** Seconds from a 429 `Retry-After` header, if present. */
export function retryAfterSeconds(err: unknown): number | null {
  const e = err as ApiError;
  if (e.response?.status !== 429) return null;
  const raw = e.response.headers?.['retry-after'];
  const secs = raw ? Number.parseInt(raw, 10) : NaN;
  return Number.isFinite(secs) && secs > 0 ? secs : null;
}

/** Digits only, at most 6: tolerates pastes like "123 - 456" or "Code: 123456". */
export function normalizeCodeInput(raw: string): string {
  return raw.replace(/\D/g, '').slice(0, 6);
}

/** The server asks for a new code (expired, used up, destination changed, none pending). */
export function needsNewCode(err: unknown): boolean {
  const e = err as ApiError;
  return e.response?.status === 400 && /request a new code/i.test(e.response.data?.detail ?? '');
}
