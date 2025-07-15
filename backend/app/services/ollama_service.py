"""
MeReader Ollama Integration Service for LLM and Embeddings
"""
import asyncio
import json
import logging
import httpx
from typing import List, Dict, Any, Optional, Union
from app.core.config import settings
from app.core.exceptions import OllamaServiceException

logger = logging.getLogger(__name__)

class OllamaService:
    """Service for interacting with locally-hosted Ollama models"""

    def __init__(self):
        self.base_url = settings.OLLAMA_BASE_URL
        self.llm_model = settings.OLLAMA_LLM_MODEL
        self.embedding_model = settings.OLLAMA_EMBEDDING_MODEL
        self.timeout = 60.0
        logger.info(f"LLM: {self.llm_model}, EM: {self.embedding_model}")

    async def _make_request(
            self,
            endpoint: str,
            data: Dict[str, Any],
            method: str = "POST",
            stream: bool = False
    ) -> Union[httpx.Response, Dict[str, Any]]:
        """
        Make a request to the Ollama API
        Args:
            endpoint: API endpoint to call
            data: Request data
            method: HTTP method (GET, POST, etc.)
            stream: Whether to stream the response
        Returns:
            Response object or parsed JSON data
        """
        url = f"{self.base_url}{endpoint}"

        try:
            async with httpx.AsyncClient(timeout=self.timeout) as client:
                if method.upper() == "POST":
                    if stream: return await client.post(url, json=data, timeout=self.timeout)
                    else: response = await client.post(url, json=data, timeout=self.timeout)
                else:
                    if stream: return await client.get(url, params=data, timeout=self.timeout)
                    else: response = await client.get(url, params=data,timeout=self.timeout)

                if response.status_code != 200:
                    error_msg = f"Ollama API error: {response.status_code} - {response.text}"
                    logger.error(error_msg)
                    raise OllamaServiceException(error_msg)

                return response.json()

        except httpx.RequestError as e:
            error_msg = f"Request to Ollama API failed: {str(e)}"
            logger.error(error_msg)
            raise OllamaServiceException(error_msg)

    async def generate_embedding(self, text: str) -> List[float]:
        """
        Generate embeddings for a text using Ollama
        Args:
            text: Input text to embed
        Returns:
            Vector embedding as floats list
        """
        try:
            data = {
                "model": self.embedding_model,
                "prompt": text,
            }
            response = await self._make_request("/api/embeddings", data)
            if "embedding" not in response: raise OllamaServiceException("No embedding found in Ollama API response")

            return response["embedding"]

        except Exception as e:
            logger.error(f"Failed to generate embedding: {str(e)}")
            raise OllamaServiceException(f"Failed to generate embedding: {str(e)}")

    async def generate_embeddings_batch(self, texts: List[str]) -> List[List[float]]:
        """
        Generate embeddings for multiple texts using Ollama
        Args:
            texts: List of input texts to embed
        Returns:
            List of vector embeddings
        """
        tasks = [self.generate_embedding(text) for text in texts]
        embeddings = await asyncio.gather(*tasks)
        return embeddings

    async def generate_completion(
            self,
            prompt: str,
            system_prompt: Optional[str] = None,
            temperature: float = 0.7,
            max_tokens: Optional[int] = None,
            stream: bool = False
    ) -> Union[str, httpx.Response]:
        """
        Generate text completion using Ollama LLM
        Args:
            prompt: Input prompt text
            system_prompt: system prompt to guide the model
            temperature: Temperature for generation (0.0 to 1.0)
            max_tokens: Maximum tokens to generate
            stream: Whether to stream the response
        Returns:
            Generated text or streaming response
        """
        try:
            data = {
                "model": self.llm_model,
                "prompt": prompt,
                "stream": stream,
                "options": { "temperature": temperature, }
            }

            if system_prompt: data["system"] = system_prompt
            if max_tokens: data["options"]["num_predict"] = max_tokens

            if stream:
                response = await self._make_request("/api/generate", data,stream=True)
                return response
            else:
                response = await self._make_request("/api/generate", data)
                return response.get("response", "")

        except Exception as e:
            logger.error(f"Failed to generate completion: {str(e)}")
            raise OllamaServiceException(f"Failed to generate completion: {str(e)}")

    async def process_streamed_response(self, response: httpx.Response) -> str:
        """
        Process a streamed response from Ollama
        Args:
            response: Streaming response from Ollama
        Returns:
            Concatenated response text
        """
        full_response = ""

        try:
            async for line in response.aiter_lines():
                if not line.strip(): continue
                try:
                    chunk = json.loads(line)
                    if "response" in chunk: full_response += chunk["response"]
                except json.JSONDecodeError: continue

            return full_response

        except Exception as e:
            logger.error(f"Failed to process streamed response: {str(e)}")
            raise OllamaServiceException(
                f"Failed to process streamed response: {str(e)}")

    async def check_status(self) -> bool:
        """
        Check if Ollama service is running
        Returns:
            True if Ollama is running and responsive
        """
        try:
            await self._make_request("/api/version", {}, method="GET")
            return True
        except Exception: return False

ollama_service = OllamaService()