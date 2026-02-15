POST http://localhost:10000/v1/chat/completions
Content-Type: application/json

{
  "messages": [
    {
      "role": "user",
      "content": "convert 100 eur"
    }
  ],
  "stream": true,
  "model": "gpt-4o"
}
HTTP 200
[Asserts]
header "content-type" matches /text\/event-stream/
body matches /^data: .*?currency_exchange.*?\n/
body matches /^data: .*?EUR.*?\n/
