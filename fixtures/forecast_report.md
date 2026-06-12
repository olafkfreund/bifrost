# GitHub Actions Importer Forecast Report

Representative capture of `gh actions-importer forecast` output, used to test the
forecast wrapper/parser (#28, #248). Re-capture from a real org to refine; the
parser keys on the "runner minutes per month" lines, the `## Total` /
`### <pipeline>` headers, and the `#### Execution time` / `#### Queue time` /
`#### Concurrent jobs` capacity sub-sections (Median / P90 / Max), so the exact
surrounding prose can vary.

## Total

- Job count: 1,200
- Estimated runner minutes per month: 23,500

#### Execution time

- Median: 4.5 minutes
- P90: 12.0 minutes
- Max: 38.0 minutes

#### Queue time

- Median: 0.8 minutes
- P90: 3.0 minutes
- Max: 11.0 minutes

#### Concurrent jobs

- Median: 2
- P90: 6
- Max: 9

## Pipeline details

### web-portal-ci
- Job count: 210
- Estimated runner minutes per month: 4,200

### payments-api-ci
- Job count: 480
- Estimated runner minutes per month: 6,800

### data-etl-ci
- Job count: 510
- Estimated runner minutes per month: 12,500
