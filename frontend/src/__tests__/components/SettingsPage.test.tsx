import { fireEvent, render, screen } from "@testing-library/react";
import SettingsPage from "../../components/SettingsPage";

describe("SettingsPage", () => {
  it("renders the preferences form and allows updating settings", () => {
    render(<SettingsPage />);

    expect(screen.getByRole("heading", { name: /preferences/i })).toBeInTheDocument();
    expect(screen.getByLabelText(/theme/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/email notifications/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/auto-save drafts/i)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/theme/i), {
      target: { value: "light" },
    });
    fireEvent.click(screen.getByLabelText(/auto-save drafts/i));
    fireEvent.click(screen.getByRole("button", { name: /save preferences/i }));

    expect(screen.getByRole("status")).toHaveTextContent(/preferences saved successfully/i);
    expect(screen.getByLabelText(/theme/i)).toHaveValue("light");
    expect(screen.getByLabelText(/auto-save drafts/i)).toBeChecked();
  });
});
