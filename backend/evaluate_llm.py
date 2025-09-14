import os
import json
import httpx
import time
import pandas as pd
import asyncio
from tqdm import tqdm
from nltk.translate.bleu_score import sentence_bleu, SmoothingFunction
from evaluate import load
import logging
import re
from collections import defaultdict

# MODELS_TO_TEST = [
#     "llama3.2:3b",
#     "llama3.2:1b",
#     "gemma3:4b",
#     "gemma3:1b",
#     "qwen3:4b"
# ]

MODELS_TO_TEST = [
    "deepseek-r1:8b",
    "deepseek-r1:1.5b", 
    "llama3.1:8b",
    "gemma3n:e4b",
    "qwen3:8b"
]

# JUDGE_MODEL = "llama3.2:3b"
JUDGE_MODEL = "gpt-oss:20b"

API_BASE_URL = "http://127.0.0.1:8000/api"
OLLAMA_API_BASE_URL = "http://localhost:11434/api"
QUERIES_FILE = "evaluation_queries.json"
SPOILER_FILE = "evaluation_spoiler.json"
RESULTS_FILE = "evaluation_results_multi_model_v3.csv"

BOOKS_TO_EVALUATE = [
    "The Death of Ivan Ilych",
    "Of Mice and Men",
    "1984",
]

logging.basicConfig(level=logging.INFO, format='%(message)s')

logging.getLogger("httpx").setLevel(logging.WARNING)
logging.getLogger("httpcore").setLevel(logging.WARNING)
logging.getLogger("httpcore.connection").setLevel(logging.WARNING)
logging.getLogger("httpcore.http11").setLevel(logging.WARNING)

bertscore_available = False
def compute_bertscore(predictions, references):
    raise RuntimeError("BERTScore not initialised")

try:
    bertscore_module = load("bertscore")
    def compute_bertscore(predictions, references):
        return bertscore_module.compute(
            predictions=predictions,
            references=references,
            lang="en",
            model_type="distilbert-base-uncased",
        )
    bertscore_available = True
    logging.info("BERTScore loaded via evaluate.load()")
except Exception as e:
    logging.warning(f"Failed to load BERTScore via evaluate.load(): {e}")
    try:
        from bert_score import score as bert_score_score
        import torch
        def compute_bertscore(predictions, references):
            P, R, F = bert_score_score(
                predictions,
                references,
                lang="en",
                model_type="distilbert-base-uncased",
                verbose=False,
            )
            # return same dict shape as evaluate
            return {
                "precision": [float(torch.mean(P).item())],
                "recall": [float(torch.mean(R).item())],
                "f1": [float(torch.mean(F).item())],
            }
        bertscore_available = True
        logging.info("BERTScore loaded via direct bert_score API")
    except Exception as e2:
        logging.error(f"Failed to load any BERTScore backend: {e2}")
        logging.error("BERTScore disabled. Install with: pip install evaluate bert-score torch")

class MeReaderEvaluator:
    def __init__(self):
        self.client = httpx.AsyncClient(timeout=1800.0)  # 30 minutes for very slow models
        self.book_ids = {}
        self.rag_system_prompt = self._get_system_prompt_from_rag_service()
        self.judge_requests = []
        self.results = []
        self.csv_flush_interval = 20
        self.generation_options = {
            "temperature": 0.3,
            "num_predict": 0,
            "num_ctx": 0
        }

    def _get_system_prompt_from_rag_service(self):
        return (
            "You are MeReader's AI assistant. Answer using only provided text. "
            "Focus on character motivations, feelings, actions, plot, mood, tone. "
            "Base answers SOLELY on facts from excerpts. "
            "Never reveal plot beyond reading progress. "
            "If info unavailable, say 'Info not available in text so far.' "
            "Answer directly, briefly, no meta text."
        )



    def dedupe_context(self, context_passages):
        if not context_passages:
            return []
        
        seen_texts = set()
        deduped = []
        
        for passage in context_passages:
            text = passage.get('text', '').strip()
            if not text or text in seen_texts:
                continue
            
            # check for near duplicates (70% similarity threshold)
            is_duplicate = False
            for seen_text in seen_texts:
                if len(text) > 50 and len(seen_text) > 50:
                    words1 = set(text.lower().split())
                    words2 = set(seen_text.lower().split())
                    similarity = len(words1 & words2) / len(words1 | words2) if words1 | words2 else 0
                    if similarity > 0.7:
                        is_duplicate = True
                        break
            
            if not is_duplicate:
                seen_texts.add(text)
                deduped.append(passage)
        
        # truncate to first 6 passages
        return deduped[:6]

    async def setup_books(self):
        print("Loading books...")
        try:
            print("GET /books")
            response = await self.client.get(f"{API_BASE_URL}/books/")
            response.raise_for_status()
            uploaded_books = {book['title'].strip().lower(): book['id'] for book in response.json().get('books', [])}
            
            for book_title_full in BOOKS_TO_EVALUATE:
                search_title = book_title_full.replace('.txt', '').strip().lower()
                if search_title in uploaded_books:
                    self.book_ids[book_title_full] = uploaded_books[search_title]
            
            if not self.book_ids:
                raise Exception("No evaluation books found")
            print(f"{len(self.book_ids)} books ready")
        except httpx.RequestError as e:
            print(f"Backend connection failed")
            raise

    async def update_progress(self, book_id, percentage):
        try:
            response = await self.client.put(f"{API_BASE_URL}/progress/{book_id}", json={"completion_percentage": percentage})
            response.raise_for_status()
        except httpx.HTTPStatusError as e:
            pass

    async def get_context_from_mereader(self, book_id, query, progress):
        start_time = time.monotonic()
        result = await self._retry_request(
            lambda: self.client.post(f"{API_BASE_URL}/query/ask/{book_id}", json={"query": query}),
            "retrieval"
        )
        
        if result["error"]:
            retrieval_result = {
                "context_used": [],
                "retrieval_time": time.monotonic() - start_time,
                "error": result["error"]
            }
        else:
            data = result["response"].json()
            context_raw = data.get('context_used', [])
            context_deduped = self.dedupe_context(context_raw)
            
            retrieval_result = {
                "context_used": context_deduped,
                "retrieval_time": time.monotonic() - start_time,
                "error": None
            }
        
        return retrieval_result

    async def _retry_request(self, request_func, operation_type, max_retries=3):
        for attempt in range(max_retries):
            try:
                response = await request_func()
                response.raise_for_status()
                return {"response": response, "error": None}

            except httpx.HTTPStatusError as e:
                error_details = ""
                try:
                    error_details = e.response.text
                except:
                    error_details = str(e)
                
                print(f"HTTP {e.response.status_code}: {error_details[:100]}")
                
                if e.response.status_code >= 500 and attempt < max_retries - 1:
                    backoff = 2 ** attempt
                    print(f"Retry {attempt+1}/{max_retries} in {backoff}s")
                    await asyncio.sleep(backoff)
                    continue
                return {"response": None, "error": f"HTTP {e.response.status_code}: {error_details[:50]}"}
                
            except (httpx.RequestError, asyncio.TimeoutError) as e:
                print(f"Request error: {str(e)[:100]}")
                if attempt < max_retries - 1:
                    backoff = 2 ** attempt
                    print(f"Retry {attempt+1}/{max_retries} in {backoff}s") 
                    await asyncio.sleep(backoff)
                    continue
                return {"response": None, "error": f"Request failed: {str(e)[:50]}"}
        
        return {"response": None, "error": "Max retries exceeded"}

    async def warm_model(self, model_name):
        payload = {
            "model": model_name,
            "prompt": "hi",
            "stream": False,
            "keep_alive": 1800  # 30 mins
        }
        
        try:
            print(f"Warming {model_name}...")
            response = await asyncio.wait_for(
                self.client.post(f"{OLLAMA_API_BASE_URL}/generate", json=payload),
                timeout=300.0  # 5 minute timeout for warm-up
            )
            response.raise_for_status()
            print(f"{model_name} warmed and kept in memory")
            return True
        except asyncio.TimeoutError:
            print(f"Warm-up timeout for {model_name}, will load on first query")
            return True 
        except Exception as e:
            print(f"Warm-up failed for {model_name}: {str(e)[:50]}")
            return True

    async def unload_model(self, model_name):
        payload = {
            "model": model_name,
            "prompt": "bye",
            "stream": False,
            "keep_alive": 0  # immediately unload
        }
        try:
            await self.client.post(f"{OLLAMA_API_BASE_URL}/generate", json=payload)
            print(f"{model_name} unloaded from memory")
        except:
            pass

    async def generate_answer_directly(self, model_name, query, context_passages, progress_percentage, keep_alive=True):
        if not context_passages:
            return {"answer": "Info not available in text so far.", "time": 0, "error": None}

        context_str = "\n".join([f"P{i+1}: {p['text']}" for i, p in enumerate(context_passages)])
        
        prompt = f"Context:\n{context_str}\n\nQ: {query}\nA:"
        
        payload = {
            "model": model_name,
            "prompt": prompt,
            "system": self.rag_system_prompt,
            "stream": False,
            "keep_alive": 1800
        }
        
        start_time = time.monotonic()
        
        try:
            response = await self.client.post(f"{OLLAMA_API_BASE_URL}/generate", json=payload)
            response.raise_for_status()
            end_time = time.monotonic()
            return {
                "answer": response.json().get('response', ''),
                "time": end_time - start_time,
                "error": None
            }
        except Exception as e:
            end_time = time.monotonic()
            print(f"Generation failed for {model_name}: {str(e)[:50]}")
            return {"answer": f"ERROR: Generation failed", "time": end_time - start_time, "error": str(e)}

    def calculate_quantitative_scores(self, generated_answer, ground_truth):
        scores = {"bleu": 0.0, "bert_precision": 0.0, "bert_recall": 0.0, "bert_f1": 0.0}
        
        if not generated_answer or not generated_answer.strip() or "ERROR:" in generated_answer: 
            return scores
            
        chencherry = SmoothingFunction()
        try:
            scores["bleu"] = sentence_bleu([ground_truth.lower().split()], generated_answer.lower().split(), smoothing_function=chencherry.method1)
        except Exception as e:
            scores["bleu"] = 0.0
        
        if bertscore_available:
            try:
                if not ground_truth.strip() or not generated_answer.strip():
                    return scores
                
                results = compute_bertscore(
                    predictions=[generated_answer],
                    references=[ground_truth],
                )
                
                precision = results.get('precision', [0.0])[0] if results.get('precision') else 0.0
                recall = results.get('recall', [0.0])[0] if results.get('recall') else 0.0
                f1 = results.get('f1', [0.0])[0] if results.get('f1') else 0.0
                
                scores.update({
                    'bert_precision': float(precision), 
                    'bert_recall': float(recall), 
                    'bert_f1': float(f1)
                })
                
            except Exception as e:
                pass
            
        return scores

    def extract_json_from_response(self, text):
        if not text or not text.strip():
            return None
            
        try:
            return json.loads(text.strip())
        except json.JSONDecodeError:
            pass
        
        import re
        
        json_pattern = r'\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\}'
        matches = re.findall(json_pattern, text)
        
        for match in matches:
            try:
                parsed = json.loads(match)
                expected_keys = {'contextual_fidelity', 'relevance', 'helpfulness', 'coherence', 'instruction_following'}
                if expected_keys.issubset(set(parsed.keys())):
                    return parsed
            except json.JSONDecodeError:
                continue
        
        code_block_pattern = r'```(?:json)?\s*(\{.*?\})\s*```'
        code_matches = re.findall(code_block_pattern, text, re.DOTALL | re.IGNORECASE)
        
        for match in code_matches:
            try:
                parsed = json.loads(match)
                expected_keys = {'contextual_fidelity', 'relevance', 'helpfulness', 'coherence', 'instruction_following'}
                if expected_keys.issubset(set(parsed.keys())):
                    return parsed
            except json.JSONDecodeError:
                continue
        
        last_brace_start = text.rfind('{')
        last_brace_end = text.rfind('}')
        
        if last_brace_start != -1 and last_brace_end != -1 and last_brace_end > last_brace_start:
            try:
                return json.loads(text[last_brace_start:last_brace_end + 1])
            except json.JSONDecodeError:
                pass
        
        return None

    def extract_spoiler_json_from_response(self, text):
        if not text or not text.strip():
            return None
            
        try:
            return json.loads(text.strip())
        except json.JSONDecodeError:
            pass
        
        import re
        
        json_pattern = r'\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\}'
        matches = re.findall(json_pattern, text)
        
        for match in matches:
            try:
                parsed = json.loads(match)
                if 'contains_spoilers' in parsed:
                    return parsed
            except json.JSONDecodeError:
                continue
        
        code_block_pattern = r'```(?:json)?\s*(\{.*?\})\s*```'
        code_matches = re.findall(code_block_pattern, text, re.DOTALL | re.IGNORECASE)
        
        for match in code_matches:
            try:
                parsed = json.loads(match)
                if 'contains_spoilers' in parsed:
                    return parsed
            except json.JSONDecodeError:
                continue
        
        last_brace_start = text.rfind('{')
        last_brace_end = text.rfind('}')
        
        if last_brace_start != -1 and last_brace_end != -1 and last_brace_end > last_brace_start:
            try:
                parsed = json.loads(text[last_brace_start:last_brace_end + 1])
                if 'contains_spoilers' in parsed:
                    return parsed
            except json.JSONDecodeError:
                pass
        
        return None

    def store_qualitative_request(self, result_index, query, generated_answer, ground_truth):
        if not generated_answer or not generated_answer.strip() or "ERROR:" in generated_answer:
            return {"contextual_fidelity": 1, "relevance": 1, "helpfulness": 1, "coherence": 1, "instruction_following": 0}
        
        prompt = f"""Score the answer vs ground truth. JSON only, no thinking.
        Score 1-5: contextual_fidelity (accuracy), relevance (addresses query), helpfulness (appropriate tone), coherence (readable)
        Score 0-1: instruction_following (no meta language like "based on text")
        Query: "{query}"
        Ground Truth: "{ground_truth}"
        Generated Answer: "{generated_answer}"
        Output: {{"contextual_fidelity": <1-5>, "relevance": <1-5>, "helpfulness": <1-5>, "coherence": <1-5>, "instruction_following": <0-1>}}
        """
        
        self.judge_requests.append({
            "type": "qualitative",
            "result_index": result_index,
            "prompt": prompt,
            "default_scores": {"contextual_fidelity": 1, "relevance": 1, "helpfulness": 1, "coherence": 1, "instruction_following": 0}
        })
        return {"contextual_fidelity": 0, "relevance": 0, "helpfulness": 0, "coherence": 0, "instruction_following": 0}

    def store_spoiler_request(self, result_index, query, generated_answer, book_title, progress_percentage):
        if not generated_answer or not generated_answer.strip():
            return 1
        try:
            with open(SPOILER_FILE, 'r', encoding='utf-8') as f:
                spoiler_rules = json.load(f)
        except FileNotFoundError:
            return 1
        
        matching_rule = None
        for rule in spoiler_rules:
            if rule['book'] == book_title and rule['query'] == query:
                matching_rule = rule
                break
        
        if not matching_rule:
            return 1
        if progress_percentage < matching_rule['percentage_of_book_read']:
            forbidden_keywords = matching_rule['evaluation_rules']['forbidden_keywords']
            prompt = f"""
            Detect spoilers. JSON only, no thinking.
            Book: {book_title}
            Progress: {progress_percentage}%
            Forbidden: {', '.join(forbidden_keywords)}
            Answer: "{generated_answer}"
            Check for direct mentions, synonyms, or hints about forbidden content.
            Output: {{"contains_spoilers": true/false, "reasoning": "brief explanation"}}
            """

            self.judge_requests.append({
                "type": "spoiler",
                "result_index": result_index,
                "prompt": prompt,
                "default_flag": 1,
                "fallback_keywords": forbidden_keywords,
                "generated_answer": generated_answer
            })
            return 0
        else:
            return 1

    def check_spoiler_prevention_basic(self, generated_answer, forbidden_keywords):
        if not generated_answer or not generated_answer.strip():
            return 1
            
        if isinstance(forbidden_keywords, list):
            answer_lower = generated_answer.lower()
            for keyword in forbidden_keywords:
                if keyword.lower() in answer_lower:
                    return 0
        return 1

    def calculate_factual_grounding(self, generated_answer, ground_truth):
        if not generated_answer or not generated_answer.strip():
            return 0
            
        if "ERROR:" in generated_answer or "information needed to answer is not available" in generated_answer.lower():
            return 5
        gt_words = set(re.findall(r'\b\w+\b', ground_truth.lower()))
        ans_words = set(re.findall(r'\b\w+\b', generated_answer.lower()))
        
        stopwords = {'a', 'an', 'the', 'is', 'it', 'in', 'on', 'of', 'for', 'to', 'and', 'but', 'was', 'were'}
        gt_meaningful = gt_words - stopwords
        ans_meaningful = ans_words - stopwords
        
        if not gt_meaningful:
            return 3
            
        overlap = len(gt_meaningful & ans_meaningful) / len(gt_meaningful)
        if overlap >= 0.7: return 5
        elif overlap >= 0.5: return 4
        elif overlap >= 0.3: return 3
        elif overlap >= 0.1: return 2
        else: return 1

    def classify_error(self, generated_answer, original_error):
        if original_error:
            return "api_error"
        elif not generated_answer or not generated_answer.strip():
            return "empty_response"
        elif "ERROR:" in generated_answer:
            return "generation_error"
        elif "information needed to answer is not available" in generated_answer.lower():
            return "insufficient_context"
        else:
            return "none"
    
    def calculate_response_complexity(self, generated_answer):
        if not generated_answer or not generated_answer.strip():
            return {"sentence_count": 0, "avg_sentence_length": 0, "unique_word_ratio": 0}
        
        import re
        sentences = re.split(r'[.!?]+', generated_answer.strip())
        sentences = [s.strip() for s in sentences if s.strip()]
        
        words = re.findall(r'\b\w+\b', generated_answer.lower())
        unique_words = set(words)
        
        return {
            "sentence_count": len(sentences),
            "avg_sentence_length": len(words) / len(sentences) if sentences else 0,
            "unique_word_ratio": len(unique_words) / len(words) if words else 0
        }
    
    def calculate_retrieval_effectiveness(self, context_used, query):
        if not context_used:
            return 0
        
        query_words = set(re.findall(r'\b\w+\b', query.lower()))
        context_text = " ".join([ctx.get('text', '') for ctx in context_used])
        context_words = set(re.findall(r'\b\w+\b', context_text.lower()))
        
        stopwords = {'a', 'an', 'the', 'is', 'it', 'in', 'on', 'of', 'for', 'to', 'and', 'but', 'was', 'were', 'with', 'by'}
        query_meaningful = query_words - stopwords
        context_meaningful = context_words - stopwords
        
        if not query_meaningful:
            return 0.5
        
        overlap = len(query_meaningful & context_meaningful) / len(query_meaningful)
        return min(overlap, 1.0)

    def flush_results_to_csv(self):
        if not self.results:
            return
        
        df = pd.DataFrame(self.results)
        mode = 'w' if not os.path.exists(RESULTS_FILE) else 'a'
        header = not os.path.exists(RESULTS_FILE)
        
        df.to_csv(RESULTS_FILE, mode=mode, header=header, index=False, encoding='utf-8')

    def save_checkpoint(self, completed_model_idx, completed_query_idx):
        checkpoint = {
            "completed_model": completed_model_idx,
            "completed_query": completed_query_idx,
            "timestamp": time.time()
        }
        with open("evaluation_checkpoint.json", "w") as f:
            json.dump(checkpoint, f)

    def load_checkpoint(self):
        try:
            with open("evaluation_checkpoint.json", "r") as f:
                return json.load(f)
        except FileNotFoundError:
            return {"completed_model": -1, "completed_query": -1}

    async def warm_judge_model(self):
        print(f"Warming judge model ({JUDGE_MODEL})...")
        payload = {
            "model": JUDGE_MODEL,
            "prompt": "ready",
            "stream": False,
            "keep_alive": 1800
        }
        try:
            print(f"POST /generate {JUDGE_MODEL} (warm)")
            response = await asyncio.wait_for(
                self.client.post(f"{OLLAMA_API_BASE_URL}/generate", json=payload),
                timeout=600.0
            )
            response.raise_for_status()
            print(f"{JUDGE_MODEL} warmed and kept in memory")
        except asyncio.TimeoutError:
            print(f"Judge warm timeout (normal for 20B models)")
        except Exception as e:
            print(f"Judge warm failed: {str(e)[:50]}")

    async def process_judge_requests(self, results, concurrency=1):
        if not self.judge_requests:
            return results
        
        if not results and os.path.exists(RESULTS_FILE):
            print("Loading results from CSV for judge processing...")
            df = pd.read_csv(RESULTS_FILE)
            results = df.to_dict('records')
            print(f"Loaded {len(results)} results from CSV")
        
        print(f"Processing {len(self.judge_requests)} judge requests with {JUDGE_MODEL}...")
        judge_start = time.time()
        
        await self.warm_judge_model()
        
        semaphore = asyncio.Semaphore(concurrency)
        
        async def process_single_request(request):
            async with semaphore:
                payload = {
                    "model": JUDGE_MODEL,
                    "prompt": request["prompt"],
                    "stream": False,
                    "keep_alive": 1800,
                    "options": {"num_predict": 64}
                }
                if request["type"] == "qualitative":
                    payload["format"] = "json"
                
                try:
                    response = await asyncio.wait_for(
                        self.client.post(f"{OLLAMA_API_BASE_URL}/generate", json=payload),
                        timeout=900.0
                    )
                    response.raise_for_status()
                    response_text = response.json()['response']
                    
                    if request["type"] == "qualitative":
                        parsed = self.extract_json_from_response(response_text)
                        return {
                            "index": request["result_index"],
                            "scores": parsed if parsed else request["default_scores"]
                        }
                    else:  # spoiler
                        parsed = self.extract_spoiler_json_from_response(response_text)
                        if parsed:
                            spoiler_detected = parsed.get('contains_spoilers', False)
                            flag = 0 if spoiler_detected else 1
                        else:
                            flag = self.check_spoiler_prevention_basic(request["generated_answer"], request["fallback_keywords"])
                        return {
                            "index": request["result_index"],
                            "flag": flag
                        }
                except asyncio.TimeoutError:
                    print(f"Judge timeout for {request['type']} request")
                    if request["type"] == "qualitative":
                        return {
                            "index": request["result_index"],
                            "scores": request["default_scores"]
                        }
                    else:
                        return {
                            "index": request["result_index"],
                            "flag": request["default_flag"]
                        }
                except Exception as e:
                    print(f"Judge error: {str(e)[:30]}")
                    if request["type"] == "qualitative":
                        return {
                            "index": request["result_index"],
                            "scores": request["default_scores"]
                        }
                    else:
                        return {
                            "index": request["result_index"],
                            "flag": request["default_flag"]
                        }
        
        tasks = [process_single_request(req) for req in self.judge_requests]
        
        with tqdm(total=len(tasks), desc="Judge", 
                 bar_format='{desc} {bar:20} {percentage:3.0f}% │ {n_fmt}/{total_fmt} │ {elapsed}<{remaining}') as pbar:
            judge_results = []
            for task in asyncio.as_completed(tasks):
                result = await task
                judge_results.append(result)
                pbar.update(1)
        
        for judge_result in judge_results:
            idx = judge_result["index"]
            if "scores" in judge_result:
                results[idx].update(judge_result["scores"])
                contextual_fidelity = judge_result["scores"].get('contextual_fidelity', 1)
                spoiler_flag = results[idx].get('spoiler_prevention_flag', 1)
                results[idx]['paas_score'] = contextual_fidelity * spoiler_flag
            else:
                results[idx]['spoiler_prevention_flag'] = judge_result["flag"]
                contextual_fidelity = results[idx].get('contextual_fidelity', 1)
                results[idx]['paas_score'] = contextual_fidelity * judge_result["flag"]
        
        judge_duration = time.time() - judge_start
        print(f"Judge complete in {judge_duration:.1f}s")
        return results

    async def run_evaluation(self):
        run_mode = input("Mode [test/full]: ").lower().strip()
        await self.setup_books()
        
        regular_queries = self._prepare_regular_queries(run_mode)
        spoiler_queries = self._prepare_spoiler_queries(run_mode)
        all_queries = regular_queries + spoiler_queries
        
        print(f"Regular queries: {len(regular_queries)}, Spoiler queries: {len(spoiler_queries)}, Total: {len(all_queries)}")
        
        if os.path.exists("evaluation_checkpoint.json"):
            os.remove("evaluation_checkpoint.json")
        if os.path.exists(RESULTS_FILE):
            os.remove(RESULTS_FILE)
        
        start_model_idx = 0  # always start from first model
        
        start_time = time.time()
        print(f"\nStarting per-model evaluation at {time.strftime('%H:%M:%S')}")
        print(f"Hardware: {os.cpu_count()} threads, no limits")
        
        model_timings = {}
        
        # outer loop: models
        for model_idx, model_name in enumerate(MODELS_TO_TEST[start_model_idx:], start_model_idx):
            model_start = time.time()
            print(f"\nModel {model_idx+1}/{len(MODELS_TO_TEST)}: {model_name}")
            
            # warm up model
            warm_success = await self.warm_model(model_name)
            if not warm_success:
                print(f"Failed to warm {model_name}, skipping")
                continue
            
            query_times = []
            
            # inner loop: queries for this model  
            with tqdm(total=len(all_queries), desc=f"Queries", 
                     bar_format='{desc} {bar:20} {percentage:3.0f}% │ {n_fmt}/{total_fmt} │ {elapsed}<{remaining}') as pbar:
                
                for query_idx, query_data in enumerate(all_queries):
                    query_start = time.time()
                    
                    book_title = query_data['book']
                    if book_title not in self.book_ids:
                        continue
                    
                    book_id = self.book_ids[book_title]
                    progress = query_data.get('progress', 100)
                    
                    await self.update_progress(book_id, progress)
                    
                    retrieval_result = await self.get_context_from_mereader(book_id, query_data['query'], progress)
                    
                    # generate answer
                    gen_result = await self.generate_answer_directly(
                        model_name, query_data['query'], retrieval_result['context_used'], progress
                    )
                    
                    # store evaluation data
                    result_index = len(self.results)
                    self._store_evaluation_result(
                        result_index, model_name, book_title, query_data, 
                        gen_result, retrieval_result, progress
                    )
                    
                    query_time = time.time() - query_start
                    query_times.append(query_time)
                    
                    pbar.set_postfix_str(f"{book_title[:10]} {query_time:.1f}s")
                    pbar.update(1)

                    # periodic flush
                    if len(self.results) % self.csv_flush_interval == 0:
                        self.flush_results_to_csv()
                        self.results = []
                    
                    self.save_checkpoint(model_idx, query_idx)
            
            # summary
            model_duration = time.time() - model_start
            avg_query_time = sum(query_times) / len(query_times) if query_times else 0
            queries_per_min = 60 / avg_query_time if avg_query_time > 0 else 0
            
            model_timings[model_name] = {
                "total_time": model_duration,
                "avg_query_time": avg_query_time,
                "queries_per_min": queries_per_min,
                "query_count": len(query_times)
            }
            
            print(f"{model_name}: {avg_query_time:.1f}s/query, {queries_per_min:.1f} q/min")
            
            await self.unload_model(model_name)
        
        self.flush_results_to_csv()
        
        final_results = await self.process_judge_requests(self.results)
        
        total_duration = time.time() - start_time
        df = pd.DataFrame(final_results)
        df = self.add_timing_columns_to_results(df, 0, 0, total_duration)
        df.to_csv(RESULTS_FILE, index=False, encoding='utf-8')
        
        mins = int(total_duration // 60)
        secs = int(total_duration % 60)
        print(f"\nComplete! {len(df)} evaluations in {mins}m {secs}s")
        print("Per-model performance:")
        for model, timing in model_timings.items():
            print(f"  {model[:15]:15} │ {timing['avg_query_time']:5.1f}s/q │ {timing['queries_per_min']:5.1f} q/min")
        
        if os.path.exists("evaluation_checkpoint.json"):
            os.remove("evaluation_checkpoint.json")
        
        self.display_summary(df)

    def _prepare_regular_queries(self, run_mode):
        with open(QUERIES_FILE, 'r', encoding='utf-8') as f:
            all_queries = json.load(f)
        
        if run_mode == 'test':
            queries_by_book = defaultdict(list)
            for q in all_queries:
                if q['book'] in self.book_ids:
                    queries_by_book[q['book']].append(q)
            
            test_queries = []
            for book_title in self.book_ids.keys():
                test_queries.extend(queries_by_book[book_title][:2])
            return [{"type": "regular", "progress": 100, **q} for q in test_queries]
        else:
            return [{"type": "regular", "progress": 100, **q} for q in all_queries if q['book'] in self.book_ids]

    def _prepare_spoiler_queries(self, run_mode):
        try:
            with open(SPOILER_FILE, 'r', encoding='utf-8') as f:
                spoiler_queries = json.load(f)
        except FileNotFoundError:
            return []
        
        available_queries = [q for q in spoiler_queries if q['book'] in self.book_ids]
        
        if run_mode == 'test':
            selected_queries = available_queries[:6]
        else:
            selected_queries = available_queries
        
        return [{"type": "spoiler", "progress": q["percentage_of_book_read"], **q} for q in selected_queries]

    def _store_evaluation_result(self, result_index, model_name, book_title, query_data, gen_result, retrieval_result, progress):
        if query_data["type"] == "regular":
            ground_truth = query_data['ground_truth']
            quant_scores = self.calculate_quantitative_scores(gen_result['answer'], ground_truth)
            factual_grounding = self.calculate_factual_grounding(gen_result['answer'], ground_truth)
            spoiler_flag = 1
            
            qual_scores = self.store_qualitative_request(result_index, query_data['query'], gen_result['answer'], ground_truth)
        else:
            quant_scores = {"bleu": 0.0, "bert_precision": 0.0, "bert_recall": 0.0, "bert_f1": 0.0}
            factual_grounding = 3
            
            qual_scores = self.store_qualitative_request(result_index, query_data['query'], gen_result['answer'], "No ground truth for spoiler evaluation")
            spoiler_flag = self.store_spoiler_request(result_index, query_data['query'], gen_result['answer'], book_title, progress)
        
        error_type = self.classify_error(gen_result['answer'], gen_result.get('error'))
        complexity_metrics = self.calculate_response_complexity(gen_result['answer'])
        retrieval_effectiveness = self.calculate_retrieval_effectiveness(retrieval_result['context_used'], query_data['query'])
        
        result = {
            "evaluation_type": query_data["type"],
            "model": model_name,
            "book": book_title,
            "query": query_data['query'],
            "progress_stage": progress,
            "generated_answer": gen_result['answer'],
            "response_time": gen_result['time'],
            "error_type": error_type,
            "context_count": len(retrieval_result['context_used']),
            "bert_precision": quant_scores['bert_precision'],
            "bert_recall": quant_scores['bert_recall'],
            "bert_f1": quant_scores['bert_f1'],
            "bleu": quant_scores['bleu'],
            "factual_grounding": factual_grounding,
            "retrieval_effectiveness": retrieval_effectiveness,
            "sentence_count": complexity_metrics['sentence_count'],
            "avg_sentence_length": complexity_metrics['avg_sentence_length'],
            "unique_word_ratio": complexity_metrics['unique_word_ratio'],
            **qual_scores,
            "paas_score": 0,
            "spoiler_prevention_flag": spoiler_flag
        }
        
        self.results.append(result)

    
    def add_timing_columns_to_results(self, df, regular_duration, spoiler_duration, total_duration):
        if df.empty:
            return df
            
        model_stats = df.groupby('model')['response_time'].agg(['count', 'sum', 'mean', 'std']).round(2)
        model_stats.columns = ['model_query_count', 'model_total_time', 'model_avg_time', 'model_time_stddev']
        
        df['evaluation_regular_duration'] = regular_duration
        df['evaluation_spoiler_duration'] = spoiler_duration  
        df['evaluation_total_duration'] = total_duration
        df['total_queries_in_run'] = len(df)
        df['avg_time_per_query'] = total_duration / len(df) if len(df) > 0 else 0
        df['queries_per_minute'] = 60 / (total_duration / len(df)) if total_duration > 0 and len(df) > 0 else 0
        
        df = df.merge(model_stats, left_on='model', right_index=True, how='left')
        
        type_stats = df.groupby('evaluation_type')['response_time'].agg(['count', 'sum', 'mean']).round(2)
        type_stats.columns = ['type_query_count', 'type_total_time', 'type_avg_time']
        df = df.merge(type_stats, left_on='evaluation_type', right_index=True, how='left')
        
        df['model_efficiency_score'] = df['model_query_count'] / df['model_total_time']
        df['relative_speed_vs_avg'] = df['response_time'] / df['model_avg_time']
        
        df['model_speed_rank'] = df.groupby('evaluation_type')['model_avg_time'].rank(method='min')
        df['response_speed_percentile'] = df.groupby('model')['response_time'].rank(pct=True) * 100
        
        return df

    def display_summary(self, df):
        if df.empty: 
            return

        print("\nModel Performance:")
        summary = df.groupby('model').agg({
            'bert_f1': 'mean',
            'paas_score': 'mean',
            'spoiler_prevention_flag': 'mean',
            'response_time': 'mean'
        }).round(3)
        
        for model in summary.index:
            f1 = summary.loc[model, 'bert_f1']
            paas = summary.loc[model, 'paas_score']
            spoiler = summary.loc[model, 'spoiler_prevention_flag']
            time_avg = summary.loc[model, 'response_time']
            print(f"  {model[:12]:12} │ F1:{f1:.3f} PAAS:{paas:.2f} Spoiler:{spoiler:.0%} Time:{time_avg:.0f}s")
        
        total_spoiler_safe = (df['spoiler_prevention_flag'] == 1).sum()
        print(f"\nSpoiler Prevention: {total_spoiler_safe}/{len(df)} ({total_spoiler_safe/len(df)*100:.0f}%)")
        print(f"Avg Quality: BERT F1 {df['bert_f1'].mean():.3f}, PAAS {df['paas_score'].mean():.2f}")

async def main():
    evaluator = MeReaderEvaluator()
    await evaluator.run_evaluation()

if __name__ == "__main__":
    asyncio.run(main())