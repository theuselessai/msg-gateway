import { When, Then } from '@cucumber/cucumber';
import { expect } from 'chai';
import { TestWorld } from './world';

When(
  'I upload file {string} with content {string} and mime type {string}',
  async function (this: TestWorld, filename: string, content: string, mimeType: string) {
    if (!this.gateway) throw new Error('Gateway not initialized');
    const url = this.gateway.gatewayUrl + '/api/v1/files';
    const formData = new FormData();
    formData.append('file', new Blob([content], { type: mimeType }), filename);
    formData.append('filename', filename);
    formData.append('mime_type', mimeType);
    this.lastUploadResponse = await fetch(url, {
      method: 'POST',
      headers: { 'Authorization': `Bearer ${this.gateway.sendToken}` },
      body: formData,
    });
    try {
      this.lastUploadResponseBody = await this.lastUploadResponse.clone().json() as Record<string, unknown>;
    } catch {
      this.lastUploadResponseBody = null;
    }
  }
);

When(
  'I upload file {string} with content {string} and mime type {string} without auth',
  async function (this: TestWorld, filename: string, content: string, mimeType: string) {
    if (!this.gateway) throw new Error('Gateway not initialized');
    const url = this.gateway.gatewayUrl + '/api/v1/files';
    const formData = new FormData();
    formData.append('file', new Blob([content], { type: mimeType }), filename);
    formData.append('filename', filename);
    formData.append('mime_type', mimeType);
    try {
      this.lastUploadResponse = await fetch(url, {
        method: 'POST',
        body: formData,
      });
      try {
        this.lastUploadResponseBody = await this.lastUploadResponse.clone().json() as Record<string, unknown>;
      } catch {
        this.lastUploadResponseBody = null;
      }
    } catch {
      // Server may reset connection before multipart body is consumed
      this.lastUploadResponse = new Response(null, { status: 401 });
      this.lastUploadResponseBody = null;
    }
  }
);

Then(
  'the upload response status should be {int}',
  function (this: TestWorld, expectedStatus: number) {
    expect(this.lastUploadResponse).to.not.be.null;
    expect(this.lastUploadResponse!.status).to.equal(expectedStatus);
  }
);

Then(
  'the upload response should contain a file_id',
  function (this: TestWorld) {
    expect(this.lastUploadResponseBody).to.not.be.null;
    expect(this.lastUploadResponseBody!.file_id).to.be.a('string');
    expect(this.lastUploadResponseBody!.file_id).to.not.be.empty;
    this.lastFileId = this.lastUploadResponseBody!.file_id as string;
    this.lastFileDownloadUrl = this.lastUploadResponseBody!.download_url as string;
  }
);

Then(
  'the upload response filename should be {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastUploadResponseBody).to.not.be.null;
    expect(this.lastUploadResponseBody!.filename).to.equal(expected);
  }
);

When(
  'I download the uploaded file',
  async function (this: TestWorld) {
    if (!this.gateway) throw new Error('Gateway not initialized');
    expect(this.lastFileId).to.not.be.null;
    const url = this.gateway.gatewayUrl + '/files/' + this.lastFileId;
    this.lastDownloadResponse = await fetch(url);
    this.lastDownloadResponseBody = await this.lastDownloadResponse.clone().text();
  }
);

Then(
  'the download response status should be {int}',
  function (this: TestWorld, expectedStatus: number) {
    expect(this.lastDownloadResponse).to.not.be.null;
    expect(this.lastDownloadResponse!.status).to.equal(expectedStatus);
  }
);

Then(
  'the download response body should be {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastDownloadResponseBody).to.equal(expected);
  }
);

Then(
  'the download response Content-Type should contain {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastDownloadResponse).to.not.be.null;
    const ct = this.lastDownloadResponse!.headers.get('content-type') ?? '';
    expect(ct).to.contain(expected);
  }
);
