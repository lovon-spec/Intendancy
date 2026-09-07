import { Link } from "react-router-dom";
import { ConnectButton } from "../wallet/ConnectButton";

export function Header() {
  return (
    <header className="border-b border-gray-200 px-6 py-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-6">
          <Link
            to="/"
            className="flex items-center gap-2 text-lg font-bold text-gray-900 hover:text-gray-700"
          >
            <img src={`${import.meta.env.BASE_URL}favicon.svg`} alt="" aria-hidden="true" className="h-7 w-7" />
            <span>Intendancy</span>
          </Link>
          <nav className="flex items-center gap-4">
            <Link to="/" className="text-sm text-gray-600 hover:text-gray-900">Registry</Link>
            <Link to="/submit" className="text-sm text-gray-600 hover:text-gray-900">Submit</Link>
            <Link to="/policy" className="text-sm text-gray-600 hover:text-gray-900">Listing Policy</Link>
          </nav>
        </div>
        <ConnectButton />
      </div>
    </header>
  );
}
