import { assertSonarrWorkloadsHealthy, expect, expectActionResponse, expectOperation, getJson, openSonarr, recordResult, test, type AppState } from './fixtures';

test('refreshes the real catalog and upgrades Sonarr from chart A to chart B', async ({ standardPage: page }) => {
  const details = await openSonarr(page);
  const initial = await getJson<AppState>(page, '/api/apps/sonarr/state');
  expect(initial.installed_chart_version).toBe(process.env.ACCEPTANCE_CHART_VERSION_A);

  const syncResponse = page.waitForResponse(response =>
    response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/apps/sync',
  );
  await page.getByRole('button', { name: 'Refresh Catalog', exact: true }).click();
  expect((await syncResponse).ok()).toBe(true);

  await expect.poll(async () => {
    const state = await getJson<AppState>(page, '/api/apps/sonarr/state');
    return { available: state.available_chart_version, update: state.update_available };
  }).toEqual({ available: process.env.ACCEPTANCE_CHART_VERSION_B, update: true });

  const update = details.getByRole('button', { name: 'Update', exact: true });
  await expect(update).toBeVisible({ timeout: 60_000 });
  await expect(details.getByText(process.env.ACCEPTANCE_CHART_VERSION_B!, { exact: true })).toBeVisible();
  const updateResponse = page.waitForResponse(response =>
    response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/apps/sonarr/update',
  );
  await update.click();
  const id = await expectActionResponse(await updateResponse);
  await expectOperation(page, id, 'update', process.env.ACCEPTANCE_CHART_VERSION_B!);
  recordResult('update_id', id);
  await assertSonarrWorkloadsHealthy(page);
});
