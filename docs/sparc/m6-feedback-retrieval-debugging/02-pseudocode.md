# M6 Feedback and Retrieval Debugging - Pseudocode

## Search / Ask Evidence
```text
execute search query
generate query_id and retrieval_run_id
persist RetrievalRunEvidence {
  query_id, retrieval_run_id, question_text, search_mode, top_k, results[]
}
print query-id and run-id with normal output
```

## Record Feedback
```text
input: query_id, optional run_id, good chunk ids, bad chunk ids, optional note
if run_id missing:
  load latest RetrievalRunEvidence for query_id
for each chunk/rating:
  load RetrievalRunEvidence by run_id
  verify evidence.query_id == query_id
  verify chunk_id is in evidence.results
  denormalize rank, score, source_path, document_id, question_text, search_mode
  save new FeedbackItem keyed by a new feedback id
```

## Generate Report
```text
load all FeedbackItems
group by query_id:
  count good/bad, compute bad_ratio, collect search modes
group by chunk_id:
  count bad ratings, count distinct queries
group by query_id:
  list queries with feedback but no good ratings
group by document_id:
  list documents with at least two distinct downvoted chunks as re-chunking candidates
print report; optionally export raw FeedbackItems as JSON
```
