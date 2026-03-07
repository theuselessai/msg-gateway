#!/usr/bin/env python3
"""
Mock Pipelit Server for testing gateway message flow.

Endpoints:
- POST /api/v1/inbound - receives messages from gateway, prints them
- GET /health - returns healthy status
"""

import json
from http.server import HTTPServer, BaseHTTPRequestHandler
from datetime import datetime


class MockPipelitHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        # Custom log format
        print(f"[{datetime.now().isoformat()}] {args[0]}")

    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"status": "healthy"}).encode())
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path == "/api/v1/inbound":
            # Read body
            content_length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(content_length)
            
            # Check auth
            auth = self.headers.get("Authorization", "")
            print(f"\n=== Inbound Message ===")
            print(f"Auth: {auth}")
            
            try:
                data = json.loads(body)
                print(f"Route: {json.dumps(data.get('route', {}))}")
                print(f"Credential: {data.get('credential_id')}")
                print(f"From: {data.get('source', {}).get('from', {})}")
                print(f"Chat: {data.get('source', {}).get('chat_id')}")
                print(f"Text: {data.get('text')}")
                attachments = data.get('attachments', [])
                if attachments:
                    print(f"Attachments: {json.dumps(attachments, indent=2)}")
                print(f"Timestamp: {data.get('timestamp')}")
                print("=" * 25)
            except json.JSONDecodeError:
                print(f"Raw body: {body.decode()}")
            
            # Return 202 Accepted
            self.send_response(202)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"status": "accepted"}).encode())
        else:
            self.send_response(404)
            self.end_headers()


def main():
    import sys
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18000
    server = HTTPServer(("127.0.0.1", port), MockPipelitHandler)
    print(f"Mock Pipelit server running on http://127.0.0.1:{port}")
    print("Endpoints:")
    print("  GET  /health")
    print("  POST /api/v1/inbound")
    print("\nWaiting for messages...\n")
    
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down...")
        server.shutdown()


if __name__ == "__main__":
    main()
