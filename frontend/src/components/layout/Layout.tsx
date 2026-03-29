import { Outlet } from "react-router-dom";
import { Header } from "./Header";

export function Layout() {
  return (
    <div className="min-h-screen bg-white">
      <Header />
      <main className="max-w-4xl mx-auto px-6 py-8">
        <Outlet />
      </main>
      <footer className="border-t border-gray-200 px-6 py-4 text-center text-xs text-gray-400">
        Intendhub - Curated by the community via Kleros on Gnosis Chain
      </footer>
    </div>
  );
}
