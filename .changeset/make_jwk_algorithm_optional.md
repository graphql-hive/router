---
router: minor
---

# Make JWK algorithm optional

Make the JWK algorithm optional as it is defined as such in the RFC. To handle a missing algorithm, we fall back to reading the algorithm from the user JWT. To protect against forged tokens, we add a validation that the algorithm in the token is part of the `allowed_algorithms`. Since `JwkMissingAlgorithm` is not longer an error, the field is removed.
