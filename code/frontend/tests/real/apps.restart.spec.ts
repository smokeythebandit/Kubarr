import { assertSonarrWorkloadsHealthy, expect, expectActionResponse, expectOperation, openSonarr, recordResult, test } from './fixtures';

test('restarts Sonarr through its real management UI and returns healthy', async ({ standardPage: page }) => {
  const details = await openSonarr(page);
  const restart = details.getByRole('button', { name: 'Restart', exact: true });
  await expect(restart, 'Installed app management must expose an accessible Restart button').toBeVisible();

  const responsePromise = page.waitForResponse(response => {
    const url = new URL(response.url());
    return response.request().method() === 'POST' && url.pathname === '/api/apps/sonarr/restart';
  });
  await restart.click();
  const id = await expectActionResponse(await responsePromise);
  await expectOperation(page, id, 'restart', process.env.ACCEPTANCE_CHART_VERSION_B!);
  recordResult('restart_id', id);
  await assertSonarrWorkloadsHealthy(page);
});
