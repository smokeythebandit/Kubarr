import { awaitOperation, credentials, expect, openSonarr, operationId, recordResult, test } from './fixtures';

test('removes Sonarr VPN and deletes the unassigned provider through the UI', async ({ authenticatedPage: page }) => {
  const fixture = credentials();
  const details = await openSonarr(page);
  const removedPromise = page.waitForResponse(response =>
    response.request().method() === 'DELETE' && new URL(response.url()).pathname === '/api/vpn/apps/sonarr');
  await details.getByRole('button', { name: 'Remove VPN', exact: true }).click();
  const removedResponse = await removedPromise;
  expect(removedResponse.ok()).toBe(true);
  const removeId = operationId(await removedResponse.json());
  await awaitOperation(page, removeId);

  await page.goto('/settings?section=vpn');
  const card = page.getByRole('heading', { name: fixture.provider_name, exact: true })
    .locator('xpath=ancestor::div[contains(@class,"border")][1]');
  page.once('dialog', dialog => dialog.accept());
  const deletedPromise = page.waitForResponse(response =>
    response.request().method() === 'DELETE' && /\/api\/vpn\/providers\/\d+$/.test(new URL(response.url()).pathname));
  await card.getByTitle('Delete provider', { exact: true }).click();
  expect((await deletedPromise).ok()).toBe(true);
  await expect(page.getByRole('heading', { name: fixture.provider_name, exact: true })).toHaveCount(0);
  recordResult({ vpn_remove_id: removeId });
});
