"use client";

import React, { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertTriangle, RefreshCw } from "lucide-react";

interface Props {
  children: ReactNode;
  fallback?: ReactNode;
  onError?: (error: Error, info: ErrorInfo) => void;
}

interface State {
  error: Error | null;
}

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.props.onError?.(error, info);
  }

  reset = () => this.setState({ error: null });

  render() {
    const { error } = this.state;
    const { children, fallback } = this.props;

    if (!error) return children;

    if (fallback) return fallback;

    return (
      <div
        role="alert"
        className="flex flex-col items-center justify-center gap-4 rounded-xl border border-rose-800/60 bg-rose-950/30 p-8 text-center"
      >
        <AlertTriangle className="h-8 w-8 text-rose-400" aria-hidden="true" />
        <div>
          <p className="text-sm font-semibold text-rose-300">
            Something went wrong
          </p>
          <p className="mt-1 text-xs text-rose-400/80">{error.message}</p>
        </div>
        <button
          onClick={this.reset}
          className="flex items-center gap-1.5 rounded-lg bg-rose-800/40 px-3 py-1.5 text-xs font-semibold text-rose-200 hover:bg-rose-800/60 transition-colors"
        >
          <RefreshCw size={12} />
          Try again
        </button>
      </div>
    );
  }
}
