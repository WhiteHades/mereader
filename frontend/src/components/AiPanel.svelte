<script lang="ts">
  import { onMount } from 'svelte';
  import { askBook, errorMessage, getAiStatus } from '../lib/commands';
  import type { AiEvent, AiStatus, AnswerResponse, SourcePassage } from '../lib/types';

  interface Props {
    bookId: string;
    open: boolean;
    onClose: () => void;
    onJumpToSource: (source: SourcePassage) => void;
  }

  let { bookId, open, onClose, onJumpToSource }: Props = $props();

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

  onMount(() => {
    void refreshStatus();
  });

  export function focusPanel(): void {
    questionInput?.focus();
  }

  async function refreshStatus(): Promise<void> {
    statusLoading = true;
    statusError = '';
    try {
      status = await getAiStatus();
    } catch (error) {
      statusError = errorMessage(error);
    } finally {
      statusLoading = false;
    }
  }

  async function submit(event?: SubmitEvent): Promise<void> {
    event?.preventDefault();
    const nextQuestion = question.trim();
    if (!nextQuestion || asking || status?.state !== 'ready') return;

    previousQuestion = nextQuestion;
    streamDraft = '';
    streamStatus = 'Finding grounded passages';
    queryError = '';
    asking = true;
    let completedFromEvent: AnswerResponse | null = null;

    try {
      const returned = await askBook(bookId, nextQuestion, (aiEvent) => {
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
      asking = false;
    }
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

  function boundaryLabel(response: AnswerResponse): string {
    const boundary = response.progressBoundary;
    if (!boundary) return 'The core did not report an indexing boundary.';
    return `Grounded through location ${boundary.currentLocation} (${Math.round(boundary.completionPercentage)}% of the book).`;
  }
</script>

<aside
  class:drawer-open={open}
  class="drawer ai-drawer"
  aria-label="Ask MeReader"
  aria-hidden={!open}
  hidden={!open}
  bind:this={panel}
>
  <header class="drawer-header">
    <div>
      <p class="eyebrow">Local book assistant</p>
      <h2>Ask MeReader</h2>
    </div>
    <button class="icon-button" onclick={onClose} aria-label="Close AI panel">Close</button>
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
        <strong>Indexing this library</strong>
        <span>{status.message || 'Questions will be available when grounded passages are ready.'}</span>
        {#if status.indexedThroughLocation !== undefined}
          <span>Indexed through location {status.indexedThroughLocation}{status.totalLocations ? ` of ${status.totalLocations}` : ''}.</span>
        {/if}
        <button class="button quiet" onclick={refreshStatus}>Refresh</button>
      </div>
    {:else if status?.state === 'error'}
      <div class="compact-state error-notice" role="alert">
        <strong>The AI core reported an error</strong>
        <span>{status.message || 'No additional detail was provided.'}</span>
        <button class="button quiet" onclick={refreshStatus}>Retry status check</button>
      </div>
    {:else}
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
        <p class="answer-copy">{answer.answer}</p>
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
            {#each answer.sources as source, index (`${source.chapterId}-${source.startLocation ?? index}`)}
              <li>
                <div class="source-heading">
                  <strong>{source.chapterTitle}</strong>
                  {#if source.relevanceScore !== undefined}
                    <span>{Math.round(source.relevanceScore * 100)}% match</span>
                  {/if}
                </div>
                <blockquote>{source.text}</blockquote>
                {#if source.startLocation !== undefined}
                  <button class="text-button" onclick={() => onJumpToSource(source)}>
                    Jump to location {source.startLocation}
                  </button>
                {/if}
              </li>
            {/each}
          </ol>
        {/if}
      </section>
    {/if}
  </div>
</aside>
