"use client";

import React, { createContext, useContext, useState, useEffect, useCallback, ReactNode } from "react";
import * as freighterApi from "@stellar/freighter-api";

export type WalletType = "freighter" | "albedo" | "xbull" | "rango" | "soroban-wallet";
export type ConnectionStatus = "idle" | "connecting" | "connected" | "error" | "unavailable";

export interface WalletAccount {
  address: string;
  name?: string;
  isMultisig?: boolean;
}

interface WalletContextType {
  activeWallet: WalletType | null;
  activeAccount: string | null;
  address: string | null; // Alias for activeAccount
  allAccounts: WalletAccount[];
  status: ConnectionStatus;
  network: string | null;
  error: string | null;
  connect: (type: WalletType, auto?: boolean) => Promise<void>;
  disconnect: () => void;
  switchAccount: (address: string) => void;
  signTransaction: (xdr: string) => Promise<string | null>;
  isWalletDetected: (type: WalletType) => boolean;
}

const WalletContext = createContext<WalletContextType | undefined>(undefined);

export function WalletProvider({ children }: { children: ReactNode }) {
  const [activeWallet, setActiveWallet] = useState<WalletType | null>(null);
  const [activeAccount, setActiveAccount] = useState<string | null>(null);
  const [allAccounts, setAllAccounts] = useState<WalletAccount[]>([]);
  const [status, setStatus] = useState<ConnectionStatus>("idle");
  const [network, setNetwork] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const isWalletDetected = useCallback((type: WalletType) => {
    if (typeof window === "undefined") return false;
    switch (type) {
      case "freighter":
        return true; // Modern Freighter uses postMessage
      case "albedo":
        return true; // Albedo is web-based or window.albedo
      case "xbull":
        // @ts-ignore
        return !!(window.xBullSDK || window.xBull);
      case "rango":
        return true; // Rango Web Suite
      case "soroban-wallet":
        // @ts-ignore
        return !!window.soroban;
      default:
        return false;
    }
  }, []);

  const connect = useCallback(async (type: WalletType, auto = false) => {
    if (typeof window === "undefined") return;

    if (!isWalletDetected(type)) {
      setStatus("unavailable");
      setError(`${type} wallet extension or service is not detected.`);
      if (!auto) {
        window.alert(`${type} wallet extension is not installed or detected. Please install or select an available wallet.`);
      }
      return;
    }

    setStatus("connecting");
    setError(null);

    try {
      let address = "";
      let net = "TESTNET";
      let fetchedAccounts: WalletAccount[] = [];

      if (type === "freighter") {
        const allowedRes = await freighterApi.isAllowed();
        let isAllowed = allowedRes.isAllowed === true;

        if (!isAllowed) {
          if (auto) {
            setStatus("idle");
            return;
          }
          const accessRes = await freighterApi.requestAccess();
          if (accessRes.error) throw new Error(accessRes.error);
          address = accessRes.address;
        } else {
          const addressRes = await freighterApi.getAddress();
          if (addressRes.error) throw new Error(addressRes.error);
          address = addressRes.address;
        }

        const networkRes = await freighterApi.getNetworkDetails();
        if (networkRes.error) throw new Error(networkRes.error);
        net = networkRes.network;
        fetchedAccounts = [{ address, name: "Freighter Main Account" }];
      } else if (type === "albedo") {
        // Albedo Link intent integration
        // @ts-ignore
        if (window.albedo && typeof window.albedo.publicKey === "function") {
          // @ts-ignore
          const res = await window.albedo.publicKey({});
          address = res.pubkey;
        } else {
          // Fallback / mock intent for web integration
          const mockAlbedoKey = "G" + Array.from({ length: 55 }, (_, i) => "ABCDEFGHJKLMNPQRSTUVWXYZ234567"[i % 30]).join("");
          address = mockAlbedoKey;
        }
        fetchedAccounts = [{ address, name: "Albedo Primary" }, { address: address.slice(0, 50) + "MULTISIG", isMultisig: true, name: "Albedo Vault (Multisig)" }];
      } else if (type === "xbull") {
        // @ts-ignore
        if (window.xBullSDK) {
          // @ts-ignore
          address = await window.xBullSDK.getPublicKey();
        } else {
          const mockXbullKey = "GXBULL" + Array.from({ length: 50 }, (_, i) => "0123456789ABCDEF"[i % 16]).join("");
          address = mockXbullKey;
        }
        fetchedAccounts = [{ address, name: "xBull Account 1" }];
      } else if (type === "rango") {
        const mockRangoKey = "GRANGO" + Array.from({ length: 50 }, (_, i) => "0123456789ABCDEF"[i % 16]).join("");
        address = mockRangoKey;
        fetchedAccounts = [{ address, name: "Rango Web Wallet" }];
      } else if (type === "soroban-wallet") {
        // @ts-ignore
        const res = await window.soroban.getPublicKey();
        address = res;
        // @ts-ignore
        net = await window.soroban.getNetwork();
        fetchedAccounts = [{ address, name: "Soroban Wallet" }];
      }

      setActiveWallet(type);
      setActiveAccount(address);
      setAllAccounts(fetchedAccounts);
      setNetwork(net);
      setStatus("connected");

      localStorage.setItem("preferred_wallet", type);
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed to connect wallet";
      setStatus("error");
      setError(msg);
      if (!auto) {
        window.alert(`Wallet Connection Error: ${msg}`);
      }
    }
  }, [isWalletDetected]);

  const disconnect = useCallback(() => {
    setActiveWallet(null);
    setActiveAccount(null);
    setAllAccounts([]);
    setNetwork(null);
    setStatus("idle");
    setError(null);
    localStorage.removeItem("preferred_wallet");
  }, []);

  const switchAccount = useCallback((address: string) => {
    setActiveAccount(address);
  }, []);

  const signTransaction = useCallback(async (xdr: string): Promise<string | null> => {
    if (!activeWallet || status !== "connected") {
      setError("No wallet connected");
      return null;
    }

    try {
      if (activeWallet === "freighter") {
        const result = await freighterApi.signTransaction(xdr, {
          networkPassphrase: network ?? "Test SDF Network ; November 2015",
        });
        return typeof result === "string" ? result : result.signedTxXdr || null;
      } else if (activeWallet === "albedo") {
        // @ts-ignore
        if (window.albedo && typeof window.albedo.tx === "function") {
          // @ts-ignore
          const res = await window.albedo.tx({ xdr, network: network ?? "TESTNET" });
          return res.signed_envelope_xdr;
        }
        return xdr; // Mock signed return
      } else if (activeWallet === "xbull") {
        // @ts-ignore
        if (window.xBullSDK) {
          // @ts-ignore
          return await window.xBullSDK.signXDR(xdr);
        }
        return xdr;
      }
      return xdr;
    } catch (err) {
      setError(err instanceof Error ? err.message : "Transaction signing failed");
      return null;
    }
  }, [activeWallet, status, network]);

  useEffect(() => {
    const preferred = localStorage.getItem("preferred_wallet") as WalletType | null;
    if (preferred && isWalletDetected(preferred)) {
      connect(preferred, true);
    }
  }, [connect, isWalletDetected]);

  return (
    <WalletContext.Provider
      value={{
        activeWallet,
        activeAccount,
        address: activeAccount,
        allAccounts,
        status,
        network,
        error,
        connect,
        disconnect,
        switchAccount,
        signTransaction,
        isWalletDetected,
      }}
    >
      {children}
    </WalletContext.Provider>
  );
}

export function useWallet() {
  const context = useContext(WalletContext);
  if (context === undefined) {
    throw new Error("useWallet must be used within a WalletProvider");
  }
  return context;
}
