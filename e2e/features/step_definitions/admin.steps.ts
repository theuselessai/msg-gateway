import { When, Then } from '@cucumber/cucumber';
import { expect } from 'chai';
import { TestWorld } from './world';

When(
  'I create a credential with id {string} and adapter {string}',
  async function (this: TestWorld, id: string, adapter: string) {
    if (!this.gateway) throw new Error('Gateway not initialized');
    const url = this.gateway.gatewayUrl + '/admin/credentials';
    this.lastResponse = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${this.gateway.adminToken}`,
      },
      body: JSON.stringify({
        id,
        adapter,
        token: 'test_token_' + id,
        active: true,
        emergency: false,
        route: { channel: 'test' },
      }),
    });
    try {
      this.lastAdminResponse = await this.lastResponse.clone().json() as Record<string, unknown>;
      this.lastResponseBody = this.lastAdminResponse;
    } catch {
      this.lastAdminResponse = null;
    }
  }
);

Then(
  'the admin response id should be {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastAdminResponse).to.not.be.null;
    expect(this.lastAdminResponse!.id).to.equal(expected);
  }
);

Then(
  'the admin response adapter should be {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastResponseBody).to.not.be.null;
    const body = this.lastResponseBody as Record<string, unknown>;
    expect(body.adapter).to.equal(expected);
  }
);

When(
  'I GET {string} with admin auth',
  async function (this: TestWorld, path: string) {
    if (!this.gateway) throw new Error('Gateway not initialized');
    const url = this.gateway.gatewayUrl + path;
    this.lastResponse = await fetch(url, {
      method: 'GET',
      headers: { 'Authorization': `Bearer ${this.gateway.adminToken}` },
    });
    try {
      this.lastResponseBody = await this.lastResponse.clone().json();
      this.lastAdminResponse = this.lastResponseBody as Record<string, unknown>;
    } catch {
      this.lastResponseBody = null;
      this.lastAdminResponse = null;
    }
  }
);

When(
  'I DELETE {string} with admin auth',
  async function (this: TestWorld, path: string) {
    if (!this.gateway) throw new Error('Gateway not initialized');
    const url = this.gateway.gatewayUrl + path;
    this.lastResponse = await fetch(url, {
      method: 'DELETE',
      headers: { 'Authorization': `Bearer ${this.gateway.adminToken}` },
    });
    try {
      this.lastResponseBody = await this.lastResponse.clone().json();
      this.lastAdminResponse = this.lastResponseBody as Record<string, unknown>;
    } catch {
      this.lastResponseBody = null;
      this.lastAdminResponse = null;
    }
  }
);

When(
  'I PATCH {string} with admin auth',
  async function (this: TestWorld, path: string) {
    if (!this.gateway) throw new Error('Gateway not initialized');
    const url = this.gateway.gatewayUrl + path;
    this.lastResponse = await fetch(url, {
      method: 'PATCH',
      headers: { 'Authorization': `Bearer ${this.gateway.adminToken}` },
    });
    try {
      this.lastResponseBody = await this.lastResponse.clone().json();
      this.lastAdminResponse = this.lastResponseBody as Record<string, unknown>;
    } catch {
      this.lastResponseBody = null;
      this.lastAdminResponse = null;
    }
  }
);

Then(
  'the response should contain a {string} array',
  function (this: TestWorld, key: string) {
    expect(this.lastResponseBody).to.not.be.null;
    const body = this.lastResponseBody as Record<string, unknown>;
    expect(body[key]).to.be.an('array');
  }
);
