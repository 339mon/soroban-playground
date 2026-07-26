"use client";

import React, { useState } from "react";
import { Terminal, Activity } from "lucide-react";
import Console from "@/components/Console";
import { EventsPanel } from "@/components/EventsPanel";

interface ConsoleAndEventsDrawerProps {
  logs: string[];
  baseLineNumber?: number;
  droppedMessages?: number;
  isIngestionPaused: boolean;
  onIngestionPauseChange: (paused: boolean) => void;
  contractId?: string;
}

export function ConsoleAndEventsDrawer({
  logs,
  baseLineNumber = 0,
  droppedMessages = 0,
  isIngestionPaused,
  onIngestionPauseChange,
  contractId,
}: ConsoleAndEventsDrawerProps) {
  const [activeTab, setActiveTab] = useState<"console" | "events">("console");

  return (
    <div className="flex flex-col rounded-xl border border-gray-800 bg-gray-950 overflow-hidden shadow-2xl">
      {/* Navigation tabs */}
      <div className="flex items-center gap-1 border-b border-gray-800 bg-gray-900 px-3 py-1.5 text-xs font-medium">
        <button
          type="button"
          onClick={() => setActiveTab("console")}
          className={`flex items-center gap-2 rounded-lg px-3 py-1.5 transition ${
            activeTab === "console"
              ? "bg-slate-800 text-cyan-400 font-semibold"
              : "text-slate-400 hover:text-slate-200 hover:bg-slate-800/50"
          }`}
        >
          <Terminal size={14} />
          <span>Console Output</span>
        </button>

        <button
          type="button"
          onClick={() => setActiveTab("events")}
          className={`flex items-center gap-2 rounded-lg px-3 py-1.5 transition ${
            activeTab === "events"
              ? "bg-slate-800 text-emerald-400 font-semibold"
              : "text-slate-400 hover:text-slate-200 hover:bg-slate-800/50"
          }`}
        >
          <Activity size={14} />
          <span>Event Streamer</span>
        </button>
      </div>

      {/* Content pane */}
      <div>
        {activeTab === "console" ? (
          <Console
            logs={logs}
            baseLineNumber={baseLineNumber}
            droppedMessages={droppedMessages}
            isIngestionPaused={isIngestionPaused}
            onIngestionPauseChange={onIngestionPauseChange}
          />
        ) : (
          <EventsPanel contractId={contractId} />
        )}
      </div>
    </div>
  );
}
