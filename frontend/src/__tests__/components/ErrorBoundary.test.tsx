import React from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import ErrorBoundary from "@/components/ErrorBoundary";

// Suppress console.error noise from intentional throws in tests
beforeEach(() => jest.spyOn(console, "error").mockImplementation(() => {}));
afterEach(() => (console.error as jest.Mock).mockRestore());

function Bomb({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) throw new Error("test explosion");
  return <div>safe content</div>;
}

describe("ErrorBoundary", () => {
  it("renders children when no error occurs", () => {
    render(
      <ErrorBoundary>
        <Bomb shouldThrow={false} />
      </ErrorBoundary>
    );
    expect(screen.getByText("safe content")).toBeInTheDocument();
  });

  it("renders default fallback UI on error", () => {
    render(
      <ErrorBoundary>
        <Bomb shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText(/something went wrong/i)).toBeInTheDocument();
    expect(screen.getByText("test explosion")).toBeInTheDocument();
  });

  it("renders custom fallback when provided", () => {
    render(
      <ErrorBoundary fallback={<div>custom fallback</div>}>
        <Bomb shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(screen.getByText("custom fallback")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("calls onError callback with error and info", () => {
    const onError = jest.fn();
    render(
      <ErrorBoundary onError={onError}>
        <Bomb shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(onError).toHaveBeenCalledWith(
      expect.objectContaining({ message: "test explosion" }),
      expect.objectContaining({ componentStack: expect.any(String) })
    );
  });

  it("recovers after clicking Try again when children no longer throw", () => {
    let shouldThrow = true;
    function Controlled() {
      if (shouldThrow) throw new Error("controlled error");
      return <div>recovered</div>;
    }

    const { rerender } = render(
      <ErrorBoundary>
        <Controlled />
      </ErrorBoundary>
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();

    shouldThrow = false;
    fireEvent.click(screen.getByRole("button", { name: /try again/i }));

    rerender(
      <ErrorBoundary>
        <Controlled />
      </ErrorBoundary>
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByText("recovered")).toBeInTheDocument();
  });

  it("does not call onError when no error occurs", () => {
    const onError = jest.fn();
    render(
      <ErrorBoundary onError={onError}>
        <Bomb shouldThrow={false} />
      </ErrorBoundary>
    );
    expect(onError).not.toHaveBeenCalled();
  });
});
