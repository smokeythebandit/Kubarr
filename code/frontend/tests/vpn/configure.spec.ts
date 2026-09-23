import { awaitOperation, credentials, expect, openSonarr, operationId, recordResult, test } from './fixtures';

test('configures custom WireGuard and assigns it to Sonarr through the UI', async ({ authenticatedPage: page }) => {
  const fixture = credentials();
  await page.goto('/settings?section=vpn');
  await page.getByRole('button', { name: /Add VPN Provider|Add Provider/, exact: false }).first().click();
  await page.getByPlaceholder('My VPN').fill(fixture.provider_name);
  await page.getByPlaceholder('Enter WireGuard private key').fill(fixture.private_key);
  await page.getByPlaceholder('10.2.0.2/32', { exact: true }).fill(fixture.addresses.join(','));
  await page.getByPlaceholder("Server's public key").fill(fixture.public_key);
  await page.getByPlaceholder('1.2.3.4').fill(fixture.endpoint_ip);
  await page.getByPlaceholder('51820').fill(String(fixture.endpoint_port));
  await page.getByPlaceholder('10.0.0.0/8,172.16.0.0/12,192.168.0.0/16')
    .fill(fixture.firewall_outbound_subnets);
  const createdPromise = page.waitForResponse(response =>
    response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/vpn/providers');
  await page.getByRole('button', { name: 'Add Provider', exact: true }).click();
  const createdResponse = await createdPromise;
  expect(createdResponse.ok()).toBe(true);
  const provider = await createdResponse.json() as { id: number };

  const details = await openSonarr(page);
  await details.getByRole('combobox').selectOption(String(provider.id));
  const assignedPromise = page.waitForResponse(response =>
    response.request().method() === 'PUT' && new URL(response.url()).pathname === '/api/vpn/apps/sonarr');
  await details.getByRole('button', { name: 'Enable VPN', exact: true }).click();
  const assignedResponse = await assignedPromise;
  expect(assignedResponse.ok()).toBe(true);
  const assignId = operationId(await assignedResponse.json());
  await awaitOperation(page, assignId);
  recordResult({ vpn_provider_id: String(provider.id), vpn_assign_id: assignId });
});
