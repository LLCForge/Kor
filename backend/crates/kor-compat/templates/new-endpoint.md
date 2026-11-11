---
name: new-endpoint
description: Add an HTTP endpoint that follows the project's existing conventions
inputs:
  - name: string # Resource or route name, for example users
  - method: enum(GET|POST|PUT|PATCH|DELETE) = GET # HTTP method
  - route: string? # Route path; derived from the name and neighbouring routes when empty
  - auth: bool = true # Require authentication like the neighbouring endpoints
steps:
  - Find how existing endpoints are declared, validated, authorized, documented and tested in this project
  - Add the {{method}} endpoint for `{{name}}` next to similar ones, with the same framework, layers and error handling
  - Validate every input at the boundary and return the project's usual error shape
  - Add tests for the success path, invalid input and, when authentication is required ({{auth}}), an unauthorized call
acceptance:
  - The project builds without new warnings
  - The new tests and the existing test suite pass
  - The endpoint is listed wherever the project documents its API (OpenAPI file, README, route table)
---
Add a {{method}} endpoint for the `{{name}}` resource.
Route path: {{route}} (when empty, derive it from the name and the neighbouring routes).
Authentication required: {{auth}}.
Match the surrounding code style and do not add new libraries.
