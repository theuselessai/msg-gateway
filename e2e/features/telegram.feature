Feature: Telegram Adapter

  @smoke
  Scenario: Inbound text message from Telegram is forwarded to backend
    Given a running gateway with Telegram adapter
    And a mock backend listening
    And a mock Telegram server running
    When a Telegram user sends text "Hello from Telegram" in chat 12345
    Then the backend should receive a message within 8000ms
    And the received message text should be "Hello from Telegram"
    And the received message source.protocol should be "telegram"
    And the received message credential_id should be "test_telegram"

  @smoke
  Scenario: Outbound text message is sent via Telegram
    Given a running gateway with Telegram adapter
    And a mock Telegram server running
    And header "Authorization" is "Bearer test_send_token"
    When I POST "/api/v1/send" with body:
      """
      {"credential_id": "test_telegram", "chat_id": "12345", "text": "Hello Telegram"}
      """
    Then the response status should be 200
    And Telegram should receive a sendMessage within 5000ms
    And the Telegram message text should be "Hello Telegram"
    And the Telegram message chat_id should be "12345"
