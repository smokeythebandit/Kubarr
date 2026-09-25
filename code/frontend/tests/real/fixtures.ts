import { chmodSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, basename, join } from 'node:path';
import { test as base, expect, type Locator, type Page, type Response } from '@playwright/test';
import {
  isOptionalCatalogIcon404,
  isOptionalCatalogIconConsoleError,
  type ConsoleErrorRecord,
} from './http-errors';

export type OperationType = 'install' | 'update' | 'restart' | 'delete';

export interface AppOperation {
  id: string;
  app_name: string;
  operation: string;
  status: string;
  message: string | null;
  error: string | null;
}

export interface AppState {
  app_name: string;
  desired_state: string;
  observed_state: string;
  healthy: boolean;
  installed_chart_version: string | null;
  available_chart_version: string | null;
  update_available: boolean;
  last_operation_id: string | null;
}

export interface ExpectedHttpError {
  status: number;
  method?: string;
  path: string | RegExp;
}

type RealFixtures = {
  page: Page;
  standardPage: Page;
  /** Register deliberate validation responses before triggering them. */
  expectedHttpErrors: ExpectedHttpError[];
};

function env(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`Missing required real-acceptance environment variable ${name}`);
  return value;
}

function redacted(value: string | null): string | null {
  if (!value) return value;
  return value
    .replace(/(password|api[_-]?key|token|secret)\s*[=:]\s*[^\s,;]+/gi, '$1=[REDACTED]')
    .replace(/(https?:\/\/)[^\s/@:]+:[^\s/@]+@/gi, '$1[REDACTED]@')
    .slice(0, 1000);
}

function matchesExpected(error: { status: number; method: string; path: string }, expected: ExpectedHttpError[]): boolean {
  const index = expected.findIndex(item =>
    item.status === error.status &&
    (!item.method || item.method.toUpperCase() === error.method) &&
    (typeof item.path === 'string' ? item.path === error.path : item.path.test(error.path)),
  );
  if (index < 0) return false;
  expected.splice(index, 1);
  return true;
}

export const test = base.extend<RealFixtures>({
  expectedHttpErrors: async ({ browserName: _browserName }, provide) => provide([]),

  page: async ({ page, context, expectedHttpErrors }, provide, testInfo) => {
    await page.goto('/login');
    const username = page.getByLabel('Username');
    await expect(username).toBeVisible({ timeout: 30_000 });
    await username.fill(env('TEST_USERNAME'));
    await page.getByLabel('Password').fill(env('TEST_PASSWORD'));
    await Promise.all([
      page.waitForURL(url => url.pathname !== '/login', { timeout: 30_000 }),
      page.getByRole('button', { name: 'Sign in', exact: true }).click(),
    ]);

    const currentUserResponse = await page.request.get('/api/users/me');
    expect(currentUserResponse.ok(), 'UI login did not establish an authenticated session').toBe(true);
    const currentUser = await currentUserResponse.json() as { username?: unknown };
    expect(currentUser.username, 'Authenticated user does not match TEST_USERNAME').toBe(env('TEST_USERNAME'));

    const origin = new URL(env('BASE_URL')).origin;
    const pageErrors: string[] = [];
    const consoleErrors: ConsoleErrorRecord[] = [];
    const failedRequests: string[] = [];
    const httpErrors: Array<{ status: number; method: string; path: string }> = [];
    const optionalIcon404Urls = new Set<string>();

    page.on('pageerror', error => pageErrors.push(redacted(error.message) || 'unknown page error'));
    page.on('console', message => {
      if (message.type() === 'error') {
        consoleErrors.push({ text: message.text(), locationUrl: message.location().url || undefined });
      }
    });
    context.on('requestfailed', request => {
      const failure = request.failure()?.errorText || 'unknown failure';
      const requestUrl = new URL(request.url());
      // A real navigation intentionally cancels in-flight React Query polling
      // from the page being left. Keep every other local or external failure.
      if (requestUrl.origin === origin && /(?:ERR_ABORTED|NS_BINDING_ABORTED)/.test(failure)) return;
      failedRequests.push(`${request.method()} ${requestUrl.pathname}: ${redacted(failure)}`);
    });
    page.on('response', response => {
      const url = new URL(response.url());
      if (url.origin === origin && response.status() >= 400) {
        const error = { status: response.status(), method: response.request().method(), path: `${url.pathname}${url.search}` };
        if (isOptionalCatalogIcon404(error)) optionalIcon404Urls.add(url.href);
        else httpErrors.push(error);
      }
    });

    await provide(page);

    // Only photograph read-only inventory pages after credential forms are gone.
    // A failed test may leave a secret-bearing form open; never capture it.
    const safeSection = new URL(page.url()).searchParams.get('section');
    if (safeSection === 'domains' &&
        await page.locator('input[type="password"], textarea, form').count() === 0) {
      await testInfo.attach('settings-inventory', { body: await page.screenshot(), contentType: 'image/png' });
    }

    // Failure evidence contains only sanitized request paths and error categories.
    // No response bodies, cookies, URLs with queries, form values or network traces.
    if (testInfo.status !== testInfo.expectedStatus) {
      await testInfo.attach('sanitized-failure', {
        body: JSON.stringify({
          path: new URL(page.url()).pathname,
          pageErrorCount: pageErrors.length,
          consoleErrorCount: consoleErrors.length,
          failedRequestCount: failedRequests.length,
          http: httpErrors.map(({ status, method, path }) => ({ status, method, path: path.split('?')[0] })),
        }),
        contentType: 'application/json',
      });
    }

    const unexpectedHttp = httpErrors.filter(error => !matchesExpected(error, expectedHttpErrors));
    expect(expectedHttpErrors, 'Registered HTTP validation errors that did not occur').toEqual([]);
    expect(unexpectedHttp, 'Unexpected same-origin HTTP errors').toEqual([]);
    expect(failedRequests, 'Failed browser requests').toEqual([]);
    expect(pageErrors, 'Uncaught page errors').toEqual([]);
    const unexpectedConsole = consoleErrors
      .filter(error => !isOptionalCatalogIconConsoleError(error, origin, optionalIcon404Urls))
      .map(error => redacted(error.text) || 'unknown console error');
    expect(unexpectedConsole, 'Unexpected console errors').toEqual([]);
  },

  standardPage: async ({ page }, provide) => provide(page),
});

export { expect };

export async function getJson<T = unknown>(page: Page, path: string): Promise<T> {
  if (!path.startsWith('/') || path.startsWith('//')) throw new Error(`GET validation path must be same-origin: ${path}`);
  const response = await page.request.get(path);
  expect(response.ok(), `GET ${path} returned ${response.status()}`).toBe(true);
  return response.json() as Promise<T>;
}

export async function expectOperation(
  page: Page,
  id: string,
  type: OperationType,
  version?: string,
): Promise<AppOperation> {
  const deadline = Date.now() + 10 * 60_000;
  let operation: AppOperation | undefined;

  while (Date.now() < deadline) {
    operation = await getJson<AppOperation>(page, `/api/apps/operations/${encodeURIComponent(id)}`);
    expect(operation.id).toBe(id);
    expect(operation.app_name).toBe('sonarr');
    expect(operation.operation).toBe(type);
    if (operation.status === 'failed') {
      throw new Error(JSON.stringify({
        id,
        type,
        status: operation.status,
        message: redacted(operation.message),
        error: redacted(operation.error),
      }));
    }
    if (operation.status === 'succeeded') break;
    expect(['queued', 'running']).toContain(operation.status);
    await page.waitForTimeout(2_000);
  }
  expect(operation?.status, `Operation ${id} did not succeed before timeout`).toBe('succeeded');

  const appName = operation!.app_name;
  const stateDeadline = Date.now() + 10 * 60_000;
  let state: AppState | undefined;
  while (Date.now() < stateDeadline) {
    state = await getJson<AppState>(page, `/api/apps/${encodeURIComponent(appName)}/state`);
    expect(state.app_name).toBe('sonarr');
    const removed = type === 'delete' && state.desired_state === 'removed' && state.observed_state === 'not_installed';
    const ready = type !== 'delete' && state.desired_state === 'installed' && state.observed_state === 'installed' && state.healthy;
    const correctVersion = !version || state.installed_chart_version === version;
    if ((removed || ready) && correctVersion && state.last_operation_id === id) return operation!;
    await page.waitForTimeout(2_000);
  }
  throw new Error(JSON.stringify({
    id,
    type,
    expectedVersion: version,
    state: state && {
      desired_state: state.desired_state,
      observed_state: state.observed_state,
      healthy: state.healthy,
      installed_chart_version: state.installed_chart_version,
      last_operation_id: state.last_operation_id,
    },
  }));
}

export async function expectActionResponse(response: Response): Promise<string> {
  expect(response.ok(), `${response.request().method()} ${new URL(response.url()).pathname} returned ${response.status()}`).toBe(true);
  const body = await response.json() as { id?: unknown };
  expect(typeof body.id).toBe('string');
  expect(body.id).not.toBe('');
  return body.id as string;
}

export function recordResult(key: string, id: string): void {
  if (!/^[a-z][a-z0-9_]*$/.test(key) || !id) throw new Error('Result key and operation id must be non-empty');
  const resultFile = env('ACCEPTANCE_RESULT_FILE');
  let result: Record<string, unknown> = {};
  try {
    result = JSON.parse(readFileSync(resultFile, 'utf8')) as Record<string, unknown>;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }
  result[key] = id;
  const temporary = join(dirname(resultFile), `.${basename(resultFile)}.${process.pid}.tmp`);
  writeFileSync(temporary, `${JSON.stringify(result, null, 2)}\n`, { mode: 0o600 });
  chmodSync(temporary, 0o600);
  renameSync(temporary, resultFile);
  chmodSync(resultFile, 0o600);
}

export function sonarrDetails(page: Page): Locator {
  return page.getByRole('region', { name: 'Sonarr details', exact: true });
}

export async function openSonarr(page: Page): Promise<Locator> {
  await page.goto('/apps');
  await expect(page.getByRole('heading', { name: 'Apps', exact: true })).toBeVisible();
  const heading = page.getByRole('heading', { name: 'Sonarr', exact: true }).first();
  await expect(heading).toBeVisible({ timeout: 60_000 });
  await heading.click();
  const details = sonarrDetails(page);
  await expect(details).toBeVisible();
  await expect(details.getByRole('heading', { name: 'Sonarr', exact: true })).toBeVisible();
  return details;
}

type ReadinessSummary = {
  healthStatus: string;
  healthHealthy: boolean;
  workloadsHealthy: boolean;
  podsAllRunningReady: boolean;
  hasSonarrPod: boolean;
  workloads: Array<{ name: string; kind: string; status: string; ready: boolean }>;
  pods: Array<{ name: string; kind: 'Pod'; status: string; ready: boolean }>;
};

function safeDiagnostic(value: string): string {
  return value.replace(/[^a-zA-Z0-9._:/-]/g, '?').slice(0, 160);
}

function readinessSummary(health: unknown, pods: unknown): ReadinessSummary {
  if (!health || typeof health !== 'object' || Array.isArray(health)) {
    throw new Error('Invalid Sonarr health payload: expected an object');
  }
  const healthRecord = health as Record<string, unknown>;
  if (typeof healthRecord.status !== 'string' || typeof healthRecord.healthy !== 'boolean' || !Array.isArray(healthRecord.deployments)) {
    throw new Error('Invalid Sonarr health payload: expected status, healthy, and deployments fields');
  }
  if (!Array.isArray(pods)) throw new Error('Invalid Sonarr pods payload: expected an array');

  const workloads = healthRecord.deployments.map((value, index) => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      throw new Error(`Invalid Sonarr workload payload at index ${index}: expected an object`);
    }
    const workload = value as Record<string, unknown>;
    if (typeof workload.name !== 'string' || !workload.name || typeof workload.kind !== 'string' || !workload.kind || typeof workload.healthy !== 'boolean') {
      throw new Error(`Invalid Sonarr workload payload at index ${index}: expected name, kind, and healthy fields`);
    }
    return {
      name: safeDiagnostic(workload.name),
      kind: safeDiagnostic(workload.kind),
      status: workload.healthy ? 'healthy' : 'unhealthy',
      ready: workload.healthy,
    };
  });

  let hasSonarrPod = false;
  let podsAllRunningReady = pods.length > 0;
  const podSummaries = pods.map((value, index) => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      throw new Error(`Invalid Sonarr pod payload at index ${index}: expected an object`);
    }
    const pod = value as Record<string, unknown>;
    if (
      typeof pod.name !== 'string' || !pod.name ||
      typeof pod.app !== 'string' ||
      typeof pod.namespace !== 'string' ||
      typeof pod.status !== 'string' ||
      typeof pod.ready !== 'boolean'
    ) {
      throw new Error(`Invalid Sonarr pod payload at index ${index}: expected name, app, namespace, status, and ready fields`);
    }
    if (pod.app === 'sonarr' && pod.name.startsWith('sonarr-')) hasSonarrPod = true;
    if (pod.namespace !== 'sonarr' || pod.status !== 'Running' || !pod.ready) podsAllRunningReady = false;
    return {
      name: safeDiagnostic(pod.name),
      kind: 'Pod' as const,
      status: safeDiagnostic(pod.status),
      ready: pod.ready,
    };
  });

  return {
    healthStatus: safeDiagnostic(healthRecord.status),
    healthHealthy: healthRecord.healthy,
    workloadsHealthy: workloads.length > 0 && workloads.every(workload => workload.ready),
    podsAllRunningReady,
    hasSonarrPod,
    workloads,
    pods: podSummaries,
  };
}

export async function assertSonarrWorkloadsHealthy(page: Page): Promise<void> {
  let finalSummary: ReadinessSummary | undefined;
  try {
    await expect.poll(async () => {
      const health = await getJson(page, '/api/apps/sonarr/health');
      const pods = await getJson(page, '/api/monitoring/pods?namespace=sonarr');
      finalSummary = readinessSummary(health, pods);
      return finalSummary;
    }, {
      message: 'Sonarr workloads and every current namespace pod must become healthy',
      timeout: 120_000,
      intervals: [1_000, 2_000, 5_000],
    }).toMatchObject({
      healthStatus: 'healthy',
      healthHealthy: true,
      workloadsHealthy: true,
      podsAllRunningReady: true,
      hasSonarrPod: true,
    });
  } catch (error) {
    const reason = error instanceof Error ? error.message : 'Unknown readiness polling failure';
    throw new Error(`${reason}\nFinal sanitized Sonarr readiness summary: ${JSON.stringify(finalSummary ?? null)}`);
  }
}
