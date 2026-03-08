Feature: Generic Adapter

  @smoke
  Scenario: REST inbound with valid auth returns 202
    Given a running gateway
    And a mock backend listening
    And header "Authorization" is "Bearer generic_token"
    When I POST "/api/v1/chat/test_generic" with body:
      """
      {"chat_id": "chat_002", "text": "REST test", "from": {"id": "u1"}}
      """
    Then the response status should be 202

  @smoke
  Scenario: REST inbound without auth returns 401
    Given a running gateway
    When I POST "/api/v1/chat/test_generic" with body:
      """
      {"chat_id": "chat_002", "text": "No auth", "from": {"id": "u1"}}
      """
    Then the response status should be 401

  @smoke
  Scenario: REST inbound with wrong token returns 401
    Given a running gateway
    And header "Authorization" is "Bearer wrong_token"
    When I POST "/api/v1/chat/test_generic" with body:
      """
      {"chat_id": "chat_002", "text": "Wrong auth", "from": {"id": "u1"}}
      """
    Then the response status should be 401

  @smoke
  Scenario: Inbound message with file attachment is forwarded with attachment info
    Given a running gateway
    And a mock backend listening
    And a mock file server running
    And header "Authorization" is "Bearer generic_token"
    When I POST "/api/v1/chat/test_generic" with body from mock file server
    Then the response status should be 202
    And the backend should receive a message within 3000ms
    And the received message should have 1 attachment
    And the attachment filename should be "test.txt"
    And the attachment mime_type should be "text/plain"
