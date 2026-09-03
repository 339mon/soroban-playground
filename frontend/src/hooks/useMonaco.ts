import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import type * as monaco from "monaco-editor";
import { scheduleEditorLoad } from "../../lib/editorLoadScheduler";
import { configureMonacoWorkers } from "../../lib/monacoWorkers";
import "monaco-editor/min/vs/editor/editor.main.css";

interface UseMonacoProps {
  language: string;
  value: string;
  onChange: (value: string) => void;
}

interface UseMonacoResult {
  containerRef: RefObject<HTMLDivElement | null>;
  isEditorReady: boolean;
}

export function useMonaco({
  language,
  value,
  onChange,
}: UseMonacoProps): UseMonacoResult {
  containerRef = useRef<HTMLDivElement | null>(null);
  editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  modelRef = useRef<monaco.editor.ITextModel | null>(null);
  workerRef = useRef<Worker | null>(null);
  onChangeRef = useRef(onChange);
  valueRef = useRef(value);
  const [isEditorReady, setIsEditorReady] = useState(false);

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    valueRef.current = value;
    const model = modelRef.current;
    if (model && value !== model.getValue()) {
      model.setValue(value);
    }
  }, [value]);

  useEffect(() => {
    let disposed = false;
    let cancel: (() => void) | undefined;
    let monacoAPI: typeof import("monaco-editor") | null = null;

    async function initEditor() {
      cancel = scheduleEditorLoad(async () => {
        while (!containerRef.current) {
          if (disposed) return;
          await new Promise((resolve) => setTimeout(resolve, 0));
        }
        if (disposed) return;

        try {
          monacoAPI = await import("monaco-editor");
          if (disposed) return;

          configureMonacoWorkers();

          const editor = monacoAPI.editor.create(containerRef.current, {
            language,
            value: valueRef.current,
            theme: "vs-dark",
            minimap: { enabled: false },
            fontSize: 14,
            padding: { top: 16, bottom: 16 },
            scrollBeyondLastLine: false,
            smoothScrolling: true,
            cursorBlinking: "smooth",
            cursorSmoothCaretAnimation: "on",
            formatOnPaste: true,
            wordWrap: "on",
            lineNumbers: "on",
            bracketPairColorization: { enabled: true },
            tabSize: 4,
            insertSpaces: true,
            renderLineHighlight: "all",
          });

          if (disposed) {
            editor.dispose();
            const model = editor.getModel();
            if (model) model.dispose();
            return;
          }

          editorRef.current = editor;
          modelRef.current = editor.getModel() ?? null;
          setIsEditorReady(true);

          const worker = new Worker(new URL("../workers/rustAnalyzer.worker.ts", import.meta.url));
          workerRef.current = worker;

          worker.onmessage = (event: MessageEvent) => {
            const { uri, diagnostics } = event.data;
            if (!modelRef.current || modelRef.current.uri.toString() !== uri) {
              return;
            }

            const markers: monaco.editor.IMarker[] = diagnostics.map((diagnostic: any) => ({
              severity:
                diagnostic.severity === "error"
                  ? monacoAPI!.MarkerSeverity.Error
                  : diagnostic.severity === "warning"
                    ? monacoAPI!.MarkerSeverity.Warning
                    : monacoAPI!.MarkerSeverity.Info,
              startLineNumber: diagnostic.startLineNumber,
              startColumn: diagnostic.startColumn,
              endLineNumber: diagnostic.endLineNumber,
              endColumn: diagnostic.endColumn,
              message: diagnostic.message,
            }));

            monacoAPI!.editor.setModelMarkers(
              modelRef.current,
              "rustAnalyzer",
              markers,
            );
          };

          editor.onDieChangeModelContent(() => {
            const currentValue = modelRef.current?.getValue();
            if (currentValue !== undefined) {
              onChangeRef.current(currentValue);
              workerRef.current?.postMessage({
                uri: modelRef.current?.uri.toString(),
                code: currentValue,
              });
            }
          });

          if (modelRef.current) {
            workerRef.current.postMessage({
              uri: modelRef.current.uri.toString(),
              code: modelRef.current.getValue(),
            });
          }
        } catch (error) {
          console.error("Failed to initialize Monaco editor", error);
        }
      });
    }

    initEditor();

    return () => {
      disposed = true;
      if (cancel) cancel();
      if (workerRef.current) {
        workerRef.current.terminate();
        workerRef.current = null;
      }
      if (editorRef.current) {
        editorRef.current.dispose();
        editorRef.current = null;
      }
      if (modelRef.current) {
        modelRef.current.dispose();
        modelRef.current = null;
      }
      setIsEditorReady(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { containerRef, isEditorReady };
}
