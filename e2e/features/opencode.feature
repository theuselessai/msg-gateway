@opencode
Feature: OpenCode Backend
  As a user messaging through a generic adapter
  I want my messages to be processed by OpenCode AI
  So that I receive AI-generated responses

  @opencode
  Scenario: Message routed to OpenCode backend and response received
    Given a gateway configured with OpenCode backend
    And a WebSocket client connected to the OpenCode generic adapter
    When I send a message "Hello AI" via the OpenCode WebSocket
    Then the OpenCode mock should receive the message
    And the WebSocket client should receive the OpenCode AI response

  @opencode
  Scenario: OpenCode session is reused for same conversation
    Given a gateway configured with OpenCode backend
    And a WebSocket client connected to the OpenCode generic adapter
    When I send a message "First message" via the OpenCode WebSocket
    And I send a message "Second message" via the OpenCode WebSocket
    Then the OpenCode mock should have created exactly 1 session
    And the OpenCode mock should have received 2 messages
