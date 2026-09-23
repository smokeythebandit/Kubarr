import { assertSonarrWorkloadsHealthy, expect, expectActionResponse, expectOperation, getJson, openSonarr, recordResult, test } from './fixtures';

test('installs Sonarr through the catalog and reaches healthy chart A workloads', async ({ standardPage: page }) => {
  const details = await openSonarr(page);
  const install = details.getByRole('button', { name: 'Install', exact: true });
  await expect(install).toBeVisible();

  const responsePromise = page.waitForResponse(response =>
    response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/apps/install',
  );
  await install.click();
  const id = await expectActionResponse(await responsePromise);
  await expectOperation(page, id, 'install', process.env.ACCEPTANCE_CHART_VERSION_A!);
  recordResult('install_id', id);

  await expect(details.getByText('Running', { exact: true })).toBeVisible({ timeout: 60_000 });
  await assertSonarrWorkloadsHealthy(page);
  const pods = await getJson<Array<{ name: string; restarts: number }>>(page, '/api/monitoring/pods?namespace=sonarr');
  const exporter = pods.find(pod => pod.name.startsWith('sonarr-exporter-'));
  expect(exporter, 'The default chart must deploy its metrics exporter').toBeDefined();
  expect(exporter!.restarts, 'Exporter must wait for generated credentials without crashing').toBe(0);
});
