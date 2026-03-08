Feature: Admin API

  @smoke
  Scenario: Admin endpoints require authorization
    Given a running gateway
    When I GET "/admin/credentials"
    Then the response status should be 401

  @smoke
  Scenario: List credentials
    Given a running gateway
    And header "Authorization" is "Bearer test_admin_token"
    When I GET "/admin/credentials"
    Then the response status should be 200
    And the response should contain a "credentials" array

  @smoke
  Scenario: Create, get, and delete a credential
    Given a running gateway
    And header "Authorization" is "Bearer test_admin_token"
    When I create a credential with id "admin_test_cred" and adapter "generic"
    Then the response status should be 201
    And the admin response id should be "admin_test_cred"
    When I GET "/admin/credentials/admin_test_cred" with admin auth
    Then the response status should be 200
    And the admin response adapter should be "generic"
    When I DELETE "/admin/credentials/admin_test_cred" with admin auth
    Then the response status should be 200
    When I GET "/admin/credentials/admin_test_cred" with admin auth
    Then the response status should be 404

  @smoke
  Scenario: Activate and deactivate a credential
    Given a running gateway
    And header "Authorization" is "Bearer test_admin_token"
    When I create a credential with id "toggle_cred" and adapter "generic"
    Then the response status should be 201
    When I PATCH "/admin/credentials/toggle_cred/deactivate" with admin auth
    Then the response status should be 200
    When I PATCH "/admin/credentials/toggle_cred/activate" with admin auth
    Then the response status should be 200
