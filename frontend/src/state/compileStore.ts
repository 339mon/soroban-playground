import { create } from 'zustand';

const MAX_ERROR_LENGTH = 500;

export type CompileErrorType = 'compilation' | 'network' | 'timeout' | 'rate-limit' | 'unknown';

export interface CompileError {
  message: string;
  type: CompileErrorType;
  code?: string;
  retryable: boolean;
}

export interface CompileProgressState {
  isCompiling: boolean;
  status: 'idle' | 'queued' | 'compiling' | 'success' | 'failed';
  message: string;
  progress: number;
  activeWorkers: number;
  maxWorkers: number;
  queueLength: number;
  estimatedWaitTimeMs: number;
  error: CompileError | null;
  // Setters
  startCompile: () => void;
  updateProgress: (payload: {
    status?: string;
    message?: string;
    progress?: number;
    activeWorkers?: number;
    maxWorkers?: number;
    queueLength?: number;
    estimatedWaitTimeMs?: number;
  }) => void;
  successCompile: (msg?: string) => void;
  failCompile: (errorMsg: string, errorType?: CompileErrorType) => void;
  reset: () => void;
}

function truncateMessage(msg: string): string {
  if (msg.length > MAX_ERROR_LENGTH) {
    return msg.slice(0, MAX_ERROR_LENGTH) + '...';
  }
  return msg;
}

function classifyError(msg: string): { type: CompileErrorType; retryable: boolean } {
  const lower = msg.toLowerCase();
  if (lower.includes('timeout') || lower.includes('timed out')) {
    return { type: 'timeout', retryable: true };
  }
  if (lower.includes('rate limit') || lower.includes('too many requests') || lower.includes('429')) {
    return { type: 'rate-limit', retryable: true };
  }
  if (
    lower.includes('network') ||
    lower.includes('econnrefused') ||
    lower.includes('enotfound') ||
    lower.includes('fetch failed')
  ) {
    return { type: 'network', retryable: true };
  }
  if (lower.includes('syntax') || lower.includes('error[E') || lower.includes('compilation')) {
    return { type: 'compilation', retryable: false };
  }
  return { type: 'unknown', retryable: false };
}

export const useCompileStore = create<CompileProgressState>((set) => ({
  isCompiling: false,
  status: 'idle',
  message: '',
  progress: 0,
  activeWorkers: 0,
  maxWorkers: 4,
  queueLength: 0,
  estimatedWaitTimeMs: 0,
  error: null,

  startCompile: () => set({
    isCompiling: true,
    status: 'queued',
    message: 'Queuing compilation...',
    progress: 0,
    error: null,
  }),

  updateProgress: (payload) => set((state) => {
    let mappedStatus = state.status;
    if (payload.status) {
      if (payload.status === 'building' || payload.status === 'compiling') {
        mappedStatus = 'compiling';
      } else if (payload.status === 'queued') {
        mappedStatus = 'queued';
      }
    }

    return {
      status: mappedStatus,
      message: payload.message ?? state.message,
      progress: payload.progress ?? state.progress,
      activeWorkers: payload.activeWorkers ?? state.activeWorkers,
      maxWorkers: payload.maxWorkers ?? state.maxWorkers,
      queueLength: payload.queueLength ?? state.queueLength,
      estimatedWaitTimeMs: payload.estimatedWaitTimeMs ?? state.estimatedWaitTimeMs,
    };
  }),

  successCompile: (msg) => set({
    isCompiling: false,
    status: 'success',
    message: msg ?? 'Compiled successfully.',
    progress: 100,
    error: null,
  }),

  failCompile: (errorMsg, errorType) => set((state) => {
    const safeMsg = truncateMessage(errorMsg || 'An unknown compilation error occurred.');
    const classified = errorType ? { type: errorType, retryable: errorType !== 'compilation' } : classifyError(safeMsg);
    return {
      isCompiling: false,
      status: 'failed',
      message: safeMsg,
      progress: state.progress > 0 ? state.progress : 0,
      error: { message: safeMsg, ...classified },
    };
  }),

  reset: () => set({
    isCompiling: false,
    status: 'idle',
    message: '',
    progress: 0,
    activeWorkers: 0,
    maxWorkers: 4,
    queueLength: 0,
    estimatedWaitTimeMs: 0,
    error: null,
  }),
}));
