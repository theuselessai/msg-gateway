Feature: File Upload and Download

  @smoke
  Scenario: Upload a file and download it back
    Given a running gateway
    And header "Authorization" is "Bearer test_send_token"
    When I upload file "hello.txt" with content "hello world" and mime type "text/plain"
    Then the upload response status should be 200
    And the upload response should contain a file_id
    And the upload response filename should be "hello.txt"
    When I download the uploaded file
    Then the download response status should be 200
    And the download response body should be "hello world"
    And the download response Content-Type should contain "text/plain"

  @smoke
  Scenario: Download a non-existent file returns 404
    Given a running gateway
    When I GET "/files/nonexistent-file-id-00000000"
    Then the response status should be 404

  @smoke
  Scenario: File upload requires authorization
    Given a running gateway
    When I upload file "test.txt" with content "data" and mime type "text/plain" without auth
    Then the upload response status should be 401
