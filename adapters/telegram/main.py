#!/usr/bin/env python3
"""
Telegram Adapter for Pipelit Gateway

Uses stdlib only (no external dependencies).
Polls Telegram API for updates and forwards to gateway.
"""

import os
import json
import sys
import signal
import threading
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.request import Request, urlopen
from urllib.error import URLError, HTTPError
from datetime import datetime, timezone

# Environment variables set by gateway
INSTANCE_ID = os.environ.get("INSTANCE_ID", "unknown")
ADAPTER_PORT = int(os.environ.get("ADAPTER_PORT", "9001"))
GATEWAY_URL = os.environ.get("GATEWAY_URL", "http://localhost:8080")
CREDENTIAL_ID = os.environ.get("CREDENTIAL_ID", "unknown")
CREDENTIAL_TOKEN = os.environ.get("CREDENTIAL_TOKEN", "")
CREDENTIAL_CONFIG = json.loads(os.environ.get("CREDENTIAL_CONFIG", "{}"))

# Telegram API base URL
TG_API_BASE = f"https://api.telegram.org/bot{CREDENTIAL_TOKEN}"

# Global state
running = True
last_update_id = 0


def log(msg):
    """Log with timestamp and instance ID"""
    print(f"[{datetime.now().isoformat()}] [{INSTANCE_ID}] {msg}", file=sys.stderr, flush=True)


def telegram_request(method, data=None, timeout=35):
    """Make a request to Telegram API"""
    url = f"{TG_API_BASE}/{method}"
    
    try:
        if data:
            req = Request(url, data=json.dumps(data).encode(), method="POST")
            req.add_header("Content-Type", "application/json")
        else:
            req = Request(url)
        
        with urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode())
    except HTTPError as e:
        body = e.read().decode() if e.fp else ""
        log(f"Telegram API error: {e.code} - {body}")
        return None
    except URLError as e:
        log(f"Telegram request failed: {e.reason}")
        return None
    except Exception as e:
        log(f"Telegram request error: {e}")
        return None


def send_to_gateway(payload):
    """Send inbound message to gateway"""
    url = f"{GATEWAY_URL}/api/v1/adapter/inbound"
    
    try:
        req = Request(url, data=json.dumps(payload).encode(), method="POST")
        req.add_header("Content-Type", "application/json")
        
        with urlopen(req, timeout=10) as resp:
            if resp.status == 202:
                return True
            else:
                log(f"Gateway returned {resp.status}")
                return False
    except Exception as e:
        log(f"Failed to send to gateway: {e}")
        return False


def get_file_url(file_id):
    """Get download URL for a Telegram file"""
    result = telegram_request("getFile", {"file_id": file_id})
    if result and result.get("ok"):
        file_path = result.get("result", {}).get("file_path")
        if file_path:
            return f"https://api.telegram.org/file/bot{CREDENTIAL_TOKEN}/{file_path}"
    return None


def handle_update(update):
    """Process a Telegram update"""
    global last_update_id
    
    update_id = update.get("update_id", 0)
    last_update_id = max(last_update_id, update_id)
    
    message = update.get("message")
    if not message:
        return
    
    # Extract message info
    chat = message.get("chat", {})
    from_user = message.get("from", {})
    text = message.get("text", "")
    caption = message.get("caption", "")  # For photos/documents
    message_id = message.get("message_id")
    
    # Check for file attachments
    file_info = None
    
    # Photo - get largest size
    if message.get("photo"):
        photos = message["photo"]
        largest = max(photos, key=lambda p: p.get("file_size", 0))
        file_id = largest.get("file_id")
        file_url = get_file_url(file_id)
        if file_url:
            file_info = {
                "url": file_url,
                "filename": f"photo_{message_id}.jpg",
                "mime_type": "image/jpeg"
            }
            text = caption or "[Photo]"
    
    # Document
    elif message.get("document"):
        doc = message["document"]
        file_id = doc.get("file_id")
        file_url = get_file_url(file_id)
        if file_url:
            file_info = {
                "url": file_url,
                "filename": doc.get("file_name", f"file_{message_id}"),
                "mime_type": doc.get("mime_type", "application/octet-stream")
            }
            text = caption or f"[Document: {doc.get('file_name', 'file')}]"
    
    # Voice message
    elif message.get("voice"):
        voice = message["voice"]
        file_id = voice.get("file_id")
        file_url = get_file_url(file_id)
        if file_url:
            file_info = {
                "url": file_url,
                "filename": f"voice_{message_id}.ogg",
                "mime_type": voice.get("mime_type", "audio/ogg")
            }
            text = "[Voice message]"
    
    # Skip if no text and no file
    if not text and not file_info:
        log(f"Skipping empty message {message_id}")
        return
    
    # Build display name
    first_name = from_user.get("first_name", "")
    last_name = from_user.get("last_name", "")
    display_name = f"{first_name} {last_name}".strip() or None
    
    # Build payload for gateway
    payload = {
        "instance_id": INSTANCE_ID,
        "chat_id": str(chat.get("id")),
        "message_id": str(message_id),
        "text": text,
        "from": {
            "id": str(from_user.get("id")),
            "username": from_user.get("username"),
            "display_name": display_name
        },
        "timestamp": datetime.now(timezone.utc).isoformat()
    }
    
    # Add file info if present
    if file_info:
        payload["file"] = file_info
        log(f"Received file from {display_name or from_user.get('username', 'unknown')}: {file_info['filename']}")
    else:
        log(f"Received message from {display_name or from_user.get('username', 'unknown')}: {text[:50]}...")
    
    if send_to_gateway(payload):
        log(f"Message {message_id} forwarded to gateway")
    else:
        log(f"Failed to forward message {message_id}")


def poll_updates():
    """Long poll for Telegram updates"""
    global running, last_update_id
    
    poll_timeout = CREDENTIAL_CONFIG.get("poll_timeout", 30)
    
    log(f"Starting Telegram polling (timeout={poll_timeout}s)")
    
    while running:
        params = {
            "timeout": poll_timeout,
            "offset": last_update_id + 1,
            "allowed_updates": ["message"]
        }
        
        result = telegram_request("getUpdates", params, timeout=poll_timeout + 5)
        
        if result and result.get("ok"):
            updates = result.get("result", [])
            for update in updates:
                try:
                    handle_update(update)
                except Exception as e:
                    log(f"Error handling update: {e}")
        elif result is None:
            # Request failed, wait before retry
            time.sleep(5)


def send_message(chat_id, text, reply_to=None):
    """Send a message to Telegram"""
    data = {
        "chat_id": chat_id,
        "text": text
    }
    
    if reply_to:
        data["reply_to_message_id"] = int(reply_to)
    
    result = telegram_request("sendMessage", data, timeout=10)
    
    if result and result.get("ok"):
        msg = result.get("result", {})
        return str(msg.get("message_id", ""))
    else:
        raise Exception(f"Failed to send message: {result}")


class AdapterHandler(BaseHTTPRequestHandler):
    """HTTP handler for adapter endpoints"""
    
    def log_message(self, format, *args):
        log(f"HTTP: {args[0]}")
    
    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "status": "healthy",
                "instance_id": INSTANCE_ID,
                "credential_id": CREDENTIAL_ID,
                "last_update_id": last_update_id
            }).encode())
        else:
            self.send_response(404)
            self.end_headers()
    
    def do_POST(self):
        if self.path == "/send":
            content_length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(content_length)
            
            try:
                data = json.loads(body)
                chat_id = data.get("chat_id")
                text = data.get("text")
                reply_to = data.get("reply_to_message_id")
                
                log(f"Sending message to chat {chat_id}: {text[:50]}...")
                
                message_id = send_message(chat_id, text, reply_to)
                
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({
                    "protocol_message_id": message_id
                }).encode())
                
            except Exception as e:
                log(f"Send error: {e}")
                self.send_response(500)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({"error": str(e)}).encode())
        else:
            self.send_response(404)
            self.end_headers()


def handle_signal(signum, frame):
    """Handle shutdown signals"""
    global running
    log(f"Received signal {signum}, shutting down...")
    running = False
    sys.exit(0)


def main():
    global running
    
    # Set up signal handlers
    signal.signal(signal.SIGTERM, handle_signal)
    signal.signal(signal.SIGINT, handle_signal)
    
    log(f"Starting Telegram adapter")
    log(f"  Port: {ADAPTER_PORT}")
    log(f"  Gateway: {GATEWAY_URL}")
    log(f"  Credential: {CREDENTIAL_ID}")
    
    # Verify bot token
    me = telegram_request("getMe")
    if me and me.get("ok"):
        bot = me.get("result", {})
        log(f"  Bot: @{bot.get('username')} ({bot.get('first_name')})")
    else:
        log("WARNING: Could not verify bot token!")
    
    # Start polling thread
    poll_thread = threading.Thread(target=poll_updates, daemon=True)
    poll_thread.start()
    
    # Start HTTP server
    server = HTTPServer(("127.0.0.1", ADAPTER_PORT), AdapterHandler)
    log(f"HTTP server listening on port {ADAPTER_PORT}")
    
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        running = False
        server.shutdown()
        log("Adapter stopped")


if __name__ == "__main__":
    main()
