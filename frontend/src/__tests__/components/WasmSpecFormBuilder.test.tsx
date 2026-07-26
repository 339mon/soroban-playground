import React from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import WasmSpecFormBuilder from "@/components/WasmSpecFormBuilder";
import { WalletProvider } from "@/components/providers/WalletProvider";

describe("WasmSpecFormBuilder", () => {
  const mockInputs = [
    { name: "to", type: "Address" },
    { name: "amount", type: "u128" },
    { name: "symbol", type: "Symbol" },
  ];

  it("renders typed parameter fields for contract method", () => {
    const values: Record<string, unknown> = {};
    const handleChange = jest.fn();

    render(
      <WalletProvider>
        <WasmSpecFormBuilder inputs={mockInputs} values={values} onChange={handleChange} />
      </WalletProvider>
    );

    expect(screen.getByText("to")).toBeInTheDocument();
    expect(screen.getByText("amount")).toBeInTheDocument();
    expect(screen.getByText("symbol")).toBeInTheDocument();
  });

  it("calls onChange when typing into input field", () => {
    const values: Record<string, unknown> = {};
    const handleChange = jest.fn();

    render(
      <WalletProvider>
        <WasmSpecFormBuilder inputs={mockInputs} values={values} onChange={handleChange} />
      </WalletProvider>
    );

    const amountInput = screen.getByPlaceholderText(/1000000000000000000/i);
    fireEvent.change(amountInput, { target: { value: "500" } });
    expect(handleChange).toHaveBeenCalledWith("amount", "500");
  });
});
