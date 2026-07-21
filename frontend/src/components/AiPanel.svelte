<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { askBook, cancelAiRequest, errorMessage, getAiStatus, reindexBook } from '../lib/commands';
  import type { AiEvent, AiStatus, AnswerResponse, SourcePassage } from '../lib/types';

  interface Props {
    bookId: string;
    open: boolean;
    modal?: boolean;
    onClose: () => void;
    onJumpToSource: (source: SourcePassage) => void;
  }

  let { bookId, open, modal = false, onClose, onJumpToSource }: Props = $props();

  let panel = $state<HTMLElement>();
  let questionInput = $state<HTMLTextAreaElement>();
  let status = $state<AiStatus | null>(null);
  let statusLoading = $state(true);
  let statusError = $state('');
  let question = $state('');
  let previousQuestion = $state('');
  let answer = $state<AnswerResponse | null>(null);
  let streamDraft = $state('');
  let streamStatus = $state('');
  let queryError = $state('');
  let asking = $state(false);
  let sourcesOpen = $state(true);
  let reindexing = $state(false);
  let reindexError = $state('');
  let activeRequestId: string | null = null;

  let canAsk = $derived(status !== null && status.state !== 'unavailable');
  let canReindex = $derived(
    status !== null &&
      status.state !== 'unavailable' &&
      status.state !== 'indexing' &&
      (
        status.state === 'error' ||
        status.embeddingModelAvailable === false ||
        (status.textOnlyBooks ?? 0) > 0
      ),
  );
  let answerSegments = $derived(answer ? parseAnswer(answer) : []);

  $effect(() => {
    if (!open) void cancelCurrentRequest();
  });

  onMount(() => {
    void refreshStatus();
  });

  onDestroy(() => {
    void cancelCurrentRequest();
  });

  export function focusPanel(): void {
    if (questionInput) questionInput.focus();
    else panel?.focus();
  }

  async function refreshStatus(): Promise<void> {
    statusLoading = true;
    statusError = '';
    try {
      status = await getAiStatus(bookId);
    } catch (error) {
      statusError = errorMessage(error);
    } finally {
      statusLoading = false;
    }
  }

  async function submit(event?: SubmitEvent): Promise<void> {
    event?.preventDefault();
    const nextQuestion = question.trim();
    if (!nextQuestion || asking || !canAsk) return;

    previousQuestion = nextQuestion;
    streamDraft = '';
    streamStatus = 'Finding grounded passages';
    queryError = '';
    asking = true;
    const requestId = crypto.randomUUID();
    activeRequestId = requestId;
    let completedFromEvent: AnswerResponse | null = null;

    try {
      const returned = await askBook(bookId, requestId, nextQuestion, (aiEvent) => {
        completedFromEvent = handleEvent(aiEvent);
      });
      const completed: AnswerResponse = completedFromEvent ?? returned;
      answer = completed;
      streamDraft = '';
      streamStatus = '';
      sourcesOpen = true;
    } catch (error) {
      queryError = errorMessage(error);
    } finally {
      if (activeRequestId === requestId) activeRequestId = null;
      asking = false;
    }
  }

  async function cancelCurrentRequest(): Promise<void> {
    const requestId = activeRequestId;
    if (!requestId) return;
    try {
      await cancelAiRequest(requestId);
    } catch {
      // The request's own result reports cancellation and transport failures.
    }
  }

  function closePanel(): void {
    void cancelCurrentRequest();
    onClose();
  }

  function handleEvent(event: AiEvent): AnswerResponse | null {
    if (event.type === 'status') {
      streamStatus = event.message;
      return null;
    }
    if (event.type === 'delta') {
      streamDraft += event.delta;
      return null;
    }
    if (event.type === 'error') {
      queryError = event.message;
      return null;
    }
    answer = event.response;
    streamDraft = '';
    sourcesOpen = true;
    return event.response;
  }

  function retryQuestion(): void {
    question = previousQuestion;
    void submit();
  }

  function clearAnswer(): void {
    question = '';
    previousQuestion = '';
    answer = null;
    streamDraft = '';
    streamStatus = '';
    queryError = '';
  }

  async function reindex(): Promise<void> {
    reindexing = true;
    reindexError = '';
    try {
      status = await reindexBook(bookId);
      await refreshStatus();
    } catch (error) {
      reindexError = errorMessage(error);
    } finally {
      reindexing = false;
    }
  }

  type AnswerSegment =
    | { type: 'text'; text: string }
    | { type: 'citation'; text: string; source: SourcePassage };

  function parseAnswer(response: AnswerResponse): AnswerSegment[] {
    const sources = new Map(response.sources.map((source) => [source.citationId, source]));
    const segments: AnswerSegment[] = [];
    const markerPattern = /\[S\d+\]/g;
    let textStart = 0;

    for (const match of response.answer.matchAll(markerPattern)) {
      const markerStart = match.index;
      if (markerStart > textStart) {
        segments.push({ type: 'text', text: response.answer.slice(textStart, markerStart) });
      }
      const marker = match[0];
      const source = sources.get(marker.slice(1, -1));
      segments.push(source
        ? { type: 'citation', text: marker, source }
        : { type: 'text', text: marker });
      textStart = markerStart + marker.length;
    }
    if (textStart < response.answer.length) {
      segments.push({ type: 'text', text: response.answer.slice(textStart) });
    }
    return segments;
  }

  function retrievalLabel(source: SourcePassage, rank: number): string {
    const methods = source.retrievalMethods.map((method) =>
      method === 'vector'
        ? 'Vector'
        : method === 'keyword' || method === 'fts'
          ? 'Keyword'
          : `${method.slice(0, 1).toLocaleUpperCase()}${method.slice(1)}`
    );
    return `Rank ${rank} / ${methods.join(' + ') || 'Grounded retrieval'}`;
  }

  function boundaryLabel(response: AnswerResponse): string {
    const boundary = response.progressBoundary;
    if (!boundary) return 'The core did not report an indexing boundary.';
    return `Grounded through location ${boundary.currentLocation} (${Math.round(boundary.completionPercentage)}% of the book).`;
  }
</script>

<div
  id="ai-panel"
  class:drawer-open={open}
  class="drawer ai-drawer"
  role="dialog"
  aria-labelledby="ai-panel-title"
  aria-modal={modal ? 'true' : undefined}
  aria-hidden={!open}
  hidden={!open}
  inert={!open}
  tabindex="-1"
  bind:this={panel}
>
  <header class="drawer-header">
    <div>
      <p class="eyebrow">Local book assistant</p>
      <h2 id="ai-panel-title">Ask MeReader</h2>
    </div>
    <button class="icon-button" onclick={closePanel} aria-label="Close AI panel">Close</button>
  </header>

  <div class="ai-body">
    {#if statusLoading}
      <div class="compact-state" role="status">
        <span class="activity-dot" aria-hidden="true"></span>
        Checking Ollama
      </div>
    {:else if statusError}
      <div class="compact-state error-notice" role="alert">
        <strong>AI status unavailable</strong>
        <span>{statusError}</span>
        <button class="button quiet" onclick={refreshStatus}>Retry status check</button>
      </div>
    {:else if status?.state === 'unavailable'}
      <div class="compact-state" role="status">
        <p class="state-code">OLLAMA / OFFLINE</p>
        <strong>Ollama is not available</strong>
        <span>{status.message || 'Start Ollama, then check again.'}</span>
        <button class="button quiet" onclick={refreshStatus}>Check again</button>
      </div>
    {:else if status?.state === 'indexing'}
      <div class="compact-state" role="status">
        <span class="activity-dot" aria-hidden="true"></span>
        <strong>Indexing this book</strong>
        <span>{status.message || 'Keyword grounding remains available while semantic passages are prepared.'}</span>
        {#if status.indexedThroughLocation !== undefined}
          <span>Indexed through location {status.indexedThroughLocation}{status.totalLocations ? ` of ${status.totalLocations}` : ''}.</span>
        {/if}
        <button class="button quiet" onclick={refreshStatus}>Refresh</button>
      </div>
    {:else if status?.state === 'error'}
      <div class="compact-state error-notice" role="alert">
        <strong>Semantic indexing needs attention</strong>
        <span>{status.message || 'Keyword grounding remains available. Reindex to retry semantic search.'}</span>
        <button class="button quiet" onclick={refreshStatus}>Retry status check</button>
      </div>
    {:else if status?.message}
      <div class="compact-state" role="status">
        <strong>Keyword grounding is ready</strong>
        <span>{status.message}</span>
      </div>
    {/if}

    {#if canReindex}
      <div class="reindex-action">
        <button class="button quiet" onclick={reindex} disabled={reindexing}>
          {reindexing ? 'Reindexing...' : 'Reindex book'}
        </button>
        <span>Retry semantic indexing for this book.</span>
      </div>
    {/if}

    {#if reindexError}
      <div class="compact-state error-notice" role="alert">
        <strong>Reindex failed</strong>
        <span>{reindexError}</span>
      </div>
    {/if}

    {#if canAsk}
      <form class="ask-form" onsubmit={submit}>
        <label for="book-question">Question about this book</label>
        <textarea
          id="book-question"
          bind:this={questionInput}
          bind:value={question}
          rows="4"
          placeholder="What does the author argue about...?"
          onkeydown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') void submit();
          }}
        ></textarea>
        <div class="form-actions">
          <span>Ctrl/Command + Enter</span>
          {#if asking}
            <button class="button quiet" type="button" onclick={cancelCurrentRequest}>Stop</button>
          {/if}
          <button class="button primary" type="submit" disabled={asking || !question.trim()}>
            {asking ? 'Answering...' : 'Ask'}
          </button>
        </div>
      </form>
    {/if}

    {#if asking}
      <section class="stream-answer" aria-live="polite" aria-busy="true">
        <p class="eyebrow">{streamStatus || 'Writing answer'}</p>
        {#if streamDraft}<p class="answer-copy">{streamDraft}</p>{/if}
      </section>
    {/if}

    {#if queryError}
      <div class="compact-state error-notice" role="alert">
        <strong>Answer failed</strong>
        <span>{queryError}</span>
        {#if previousQuestion}
          <button class="button quiet" onclick={retryQuestion} disabled={asking}>Retry question</button>
        {/if}
      </div>
    {/if}

    {#if answer}
      <section class="answer" aria-label="AI answer">
        <button class="text-button" type="button" onclick={clearAnswer}>Clear answer</button>
        <p class="answer-copy">
          {#each answerSegments as segment, index (`${segment.type}-${index}`)}
            {#if segment.type === 'citation'}
              <button
                class="citation-button"
                type="button"
                aria-label={`Jump to source ${segment.source.citationId}`}
                onclick={() => onJumpToSource(segment.source)}
              >{segment.text}</button>
            {:else}{segment.text}{/if}
          {/each}
        </p>
        <p class="grounding-boundary">{boundaryLabel(answer)}</p>

        <button
          class="source-disclosure"
          onclick={() => (sourcesOpen = !sourcesOpen)}
          aria-expanded={sourcesOpen}
          aria-controls="answer-sources"
        >
          <span>Grounded sources</span>
          <span>{answer.sources.length} {sourcesOpen ? 'Hide' : 'Show'}</span>
        </button>

        {#if sourcesOpen}
          <ol class="source-list" id="answer-sources">
            {#each answer.sources as source, index (source.citationId)}
              <li>
                <div class="source-heading">
                  <div>
                    <span class="source-citation">{source.citationId}</span>
                    <strong>{source.chapterTitle}</strong>
                  </div>
                  <span>{retrievalLabel(source, index + 1)}</span>
                </div>
                <blockquote>{source.text}</blockquote>
                {#if source.startLocation !== undefined}
                  <button class="text-button" onclick={() => onJumpToSource(source)}>
                    Jump to source {source.citationId} at location {source.startLocation}
                  </button>
                {/if}
              </li>
            {/each}
          </ol>
        {/if}
      </section>
    {/if}
  </div>
</div>
