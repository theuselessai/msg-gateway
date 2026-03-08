Feature: Gateway Core

  @smoke
  Scenario: Health endpoint returns 200
    Given a running gateway
    When I GET "/health"
    Then the response status should be 200

  @smoke
  Scenario: Inbound message via generic adapter is routed to backend
    Given a running gateway
    And a mock backend listening
    And header "Authorization" is "Bearer generic_token"
    When I POST "/api/v1/chat/test_generic" with body:
      """
      {"chat_id": "chat_001", "text": "Hello E2E", "from": {"id": "user_001"}}
      """
    Then the response status should be 202
    And the backend should receive a message within 3000ms
    And the received message text should be "Hello E2E"
    And the received message credential_id should be "test_generic"
    And the received message source.protocol should be "generic"

  @smoke
  Scenario: Outbound message reaches WebSocket subscriber
    Given a running gateway
    And a WebSocket client connected to "/ws/chat/test_generic/chat_001" with token "generic_token"
    And header "Authorization" is "Bearer test_send_token"
    When I POST "/api/v1/send" with body:
      """
      {"credential_id": "test_generic", "chat_id": "chat_001", "text": "Hello WS"}
      """
    Then the response status should be 200
    And the WebSocket client should receive a message with text "Hello WS" within 3000ms

  @smoke
  Scenario: Send endpoint requires authorization
    Given a running gateway
    When I POST "/api/v1/send" with body:
      """
      {"credential_id": "test_generic", "chat_id": "chat_001", "text": "No auth"}
      """
    Then the response status should be 401

  @smoke
  Scenario: Health endpoint returns status ok in body
    Given a running gateway
    When I GET "/health"
    Then the response status should be 200
    And the response body status should be "ok"

  @smoke
  Scenario: Send to unknown credential returns 404
    Given a running gateway
    And header "Authorization" is "Bearer test_send_token"
    When I POST "/api/v1/send" with body:
      """
      {"credential_id": "nonexistent_cred", "chat_id": "chat1", "text": "hello"}
      """
    Then the response status should be 404
