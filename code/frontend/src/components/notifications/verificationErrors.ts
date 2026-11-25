// Helpers for reading axios errors from the verification endpoints.

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
