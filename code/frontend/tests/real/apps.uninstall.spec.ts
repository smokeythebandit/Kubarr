import { expect, expectActionResponse, expectOperation, getJson, openSonarr, recordResult, test } from './fixtures';

test('uninstalls Sonarr through the real UI and observes removal', async ({ standardPage: page }) => {
  const details = await openSonarr(page);
  page.once('dialog', async dialog => {
    if (!/(uninstall|remove).*sonarr|sonarr.*(uninstall|remove)/i.test(dialog.message())) {
      await dialog.dismiss();
      throw new Error(`Unexpected confirmation dialog: ${dialog.message()}`);
    }
    await dialog.accept();
  });

  const responsePromise = page.waitForResponse(response =>
    response.request().method() === 'DELETE' && new URL(response.url()).pathname === '/api/apps/sonarr',
  );
  await details.getByRole('button', { name: 'Uninstall', exact: true }).click();
  const id = await expectActionResponse(await responsePromise);
  await expectOperation(page, id, 'delete');
  recordResult('delete_id', id);

  const exists = await getJson<{ exists: boolean }>(page, '/api/apps/sonarr/exists');
  expect(exists.exists).toBe(false);
  const availableDetails = await openSonarr(page);
  await expect(availableDetails.getByRole('button', { name: 'Install', exact: true })).toBeVisible({ timeout: 60_000 });
});
