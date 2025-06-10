# Audits

This directory contains various test cases for auditing GraphQL Federation and GraphQL over HTTP.

## Installing Dependencies
Make sure you have Node.js installed, then run:

```bash
npm install
```

## Running Federation Audits

Run all the test cases;

```bash
npm run test:federation-all
```

Run a specific test case (e.g., `complex-entity-call`):
```bash
npm run test:federation-single -- --test=complex-entity-call
```

You can find the logs in the `logs` directory.
You can also find JUnit XML reports in the `reports` directory.

## Running GraphQL Over HTTP Tests

```bash
npm run test:graphql-over-http
```