import { formatEther } from "viem";

export function formatXdai(value: bigint): string {
  const formatted = formatEther(value);
  const num = parseFloat(formatted);
  if (num === 0) return "0 xDAI";
  if (num < 0.001) return "<0.001 xDAI";
  return `${num.toFixed(3)} xDAI`;
}

export function truncateAddress(address: string): string {
  return `${address.slice(0, 6)}...${address.slice(-4)}`;
}

export function timeRemaining(targetTimestamp: bigint): string {
  const now = BigInt(Math.floor(Date.now() / 1000));
  if (targetTimestamp <= now) return "Expired";
  const diff = Number(targetTimestamp - now);
  const days = Math.floor(diff / 86400);
  const hours = Math.floor((diff % 86400) / 3600);
  const minutes = Math.floor((diff % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}
