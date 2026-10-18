# Exercise coach model choice

Decision reviewed on 2026-10-18. The priority is a useful assessment shortly after a completed handwritten step, with neutral feedback when the reading is uncertain. No model has yet passed a representative handwriting evaluation in Inkarchy; the defaults are candidates, not a claim of proven accuracy or real-time performance.

| Candidate | Fit for Inkarchy | Decision |
| --- | --- | --- |
| Gemini 3.5 Flash-Lite, API | Image input, structured output, and a design focused on low latency and document parsing. | First candidate for fast feedback. The integration uses minimal thinking and a short structured answer; it still needs measurements with a user-supplied API key. |
| Qwen3-VL 4B Instruct, Ollama | OCR and visual STEM capabilities in a locally runnable model. | Optional offline candidate. Use the explicit Instruct tag. Hardware is the limiting factor for continuous feedback. |
| Ministral 3 3B | Small model with vision and structured output, intended for local/edge deployment. | A possible alternative to benchmark, but its size alone does not establish better handwritten maths accuracy or lower latency on this machine. |

Capability sources: [Gemini model documentation](https://ai.google.dev/gemini-api/docs/models/gemini-3.5-flash-lite), [Qwen's model card](https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct), [Mistral's model documentation](https://docs.mistral.ai/models/ministral-3-3b-25-12). The selection above is an engineering judgement, not a comparative benchmark result.

## Why one vision request

The same request reads the handwritten expression and assesses the completed step in context. This preserves the layout of fractions, exponents and units and avoids a second model invocation. Subsequent requests include previous context and a crop around the answer. A dedicated OCR model followed by a text evaluator may become useful, but it introduces another source of latency and recognition errors; it needs a measured advantage before adding that complexity.

The UI never waits for inference. It sends completed strokes after a 350 ms pause, limits new requests to once every 1.2 seconds, and coalesces changes while one request runs. These are scheduling limits, not promised response times. Neutral feedback is preferable to a confident judgement about ambiguous ink.

## What has actually been measured

On an i7-1360P with 16 GB RAM, Ollama reported CPU-only execution. A synthetic exercise with the installed `qwen3-vl:2b` variant took about 159 seconds for inference: approximately 104 seconds of prompt/image processing and 55 seconds of generation. It exhausted the 384-token response budget in its thinking output and produced no usable verdict despite `think: false`.

This is evidence against that exact local configuration for near-live feedback. It is not a benchmark of `qwen3-vl:4b-instruct`, and does not establish its accuracy or speed. The Instruct download was interrupted before installation; no further model downloads were made during the selection review. No live Gemini request has been tested because no API key was supplied. Offline tests cover scheduling, cancellation, image preparation, provider contracts and UI behaviour, not model quality.

## Validation before calling a model fast or reliable

Use consented or synthetic exercises covering correct and incorrect algebra steps, fractions and signs, physical units, valid alternative methods, incomplete formulas, ambiguous handwriting, corrections, ordinary notes and imported statements. Check the faithful transcription separately from the verdict, especially false error reports on correct work.

Measure complete request latency (including image preparation and network), cold versus warm runs, median and 95th percentile, and how often the coach remains neutral. A 1–2 second response is a target to test for the API candidate, not a guarantee. Do not switch providers or upload a notebook to obtain a faster result without the user's selected API configuration and notebook opt-in.
