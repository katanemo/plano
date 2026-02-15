POST http://localhost:10000/v1/chat/completions
Content-Type: application/json

{
  "messages": [
    {
      "role": "user",
      "content": "how is the weather in seattle for next 5 days"
    }
  ]
}
HTTP 200
[Asserts]
header "content-type" == "application/json"
jsonpath "$.model" matches /^gpt-4o/
jsonpath "$.metadata.x-arch-state" != null
jsonpath "$.usage" != null
jsonpath "$.choices[0].message.content" matches /Seattle/
jsonpath "$.choices[0].message.role" == "assistant"
