import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";

const mockMonacoEditorStub = jest.fn(({ value, onChange }) => (
  <div data-testid="monaco-editor">
    <div>{value}</div>
    <button type="button" onClick={() => onChange?.("updated code")}>
      Change code
    </button>
  </div>
));

jest.mock("@/lib/monacoWorkers", () => ({
  configureMonacoWorkers: jest.fn(),
}));

jest.mock("@/lib/editorLoadScheduler", () => ({
  scheduleEditorLoad: jest.fn((task) => {
    void task();
    return jest.fn();
  }),
  loadMonacoEditor: jest.fn(async () => ({ default: mockMonacoEditorStub })),
}));

jest.mock("@/hooks/useCollaborativeEditor", () => ({
  useCollaborativeEditor: jest.fn(),
}));

import Editor from "../../components/Editor";
import { configureMonacoWorkers } from "@/lib/monacoWorkers";
import {
  loadMonacoEditor,
  scheduleEditorLoad,
} from "@/lib/editorLoadScheduler";
import { useCollaborativeEditor } from "@/hooks/useCollaborativeEditor";

describe("Editor", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (useCollaborativeEditor as jest.Mock).mockReturnValue({
      peers: [],
      isConnected: false,
      sendCursorUpdate: jest.fn(),
    });
  });

  it("renders the loading state and then renders the Monaco editor", async () => {
    const setCode = jest.fn();

    render(<Editor code="initial code" setCode={setCode} />);

    expect(screen.getByText(/loading editor/i)).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("monaco-editor")).toBeInTheDocument(),
    );

    expect(configureMonacoWorkers).toHaveBeenCalledTimes(1);
    expect(scheduleEditorLoad).toHaveBeenCalledTimes(1);
    expect(loadMonacoEditor).toHaveBeenCalledTimes(1);
    expect(screen.getByText("initial code")).toBeInTheDocument();
  });

  it("calls setCode when the Monaco editor onChange is invoked", async () => {
    const setCode = jest.fn();

    render(<Editor code="initial code" setCode={setCode} />);

    await waitFor(() =>
      expect(screen.getByTestId("monaco-editor")).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("button", { name: /change code/i }));

    expect(setCode).toHaveBeenCalledWith("updated code");
  });

  it("renders collaborative header with peer count and connection state", async () => {
    (useCollaborativeEditor as jest.Mock).mockReturnValue({
      peers: [{ id: "peer-1", name: "Peer", color: "#ff0000" }],
      isConnected: true,
      sendCursorUpdate: jest.fn(),
    });

    render(<Editor code="initial code" setCode={jest.fn()} />);

    await waitFor(() =>
      expect(screen.getByTestId("monaco-editor")).toBeInTheDocument(),
    );

    expect(screen.getByText(/Collab \(2\)/i)).toBeInTheDocument();
    expect(screen.queryByText(/Connected/i)).not.toBeInTheDocument();
  });
});
