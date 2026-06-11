import React, { useState } from 'react';
import { QRCodeSVG } from 'qrcode.react';
import { Invite } from '../../api/users';

interface InviteLinkModalProps {
  invite: Invite;
  onClose: () => void;
}

const InviteLinkModal: React.FC<InviteLinkModalProps> = ({ invite, onClose }) => {
  const [copied, setCopied] = useState(false);

  const inviteUrl = `${window.location.origin}/auth/register?invite=${invite.code}`;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(inviteUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  };

  const handleBackdropClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget) {
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"
      onClick={handleBackdropClick}
    >
      <div className="bg-gray-800 rounded-lg border border-gray-700 p-6 max-w-md w-full mx-4 shadow-xl">
        <div className="flex justify-between items-center mb-4">
          <h2 className="text-xl font-semibold text-white">Invite Created</h2>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-white transition-colors"
            aria-label="Close"
          >
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <p className="text-gray-400 text-sm mb-4">
          Share this link with someone to invite them to register.
        </p>

        <div className="mb-6">
          <label className="block text-sm font-medium text-gray-300 mb-2">Invite Link</label>
          <div className="flex gap-2">
            <input
              type="text"
              readOnly
              value={inviteUrl}
              className="flex-1 bg-gray-900 border border-gray-600 rounded-md px-3 py-2 text-sm text-gray-300 focus:outline-none focus:border-blue-500"
            />
            <button
              onClick={handleCopy}
              className={`px-4 py-2 rounded-md font-medium transition-colors ${
                copied
                  ? 'bg-green-600 text-white'
                  : 'bg-blue-600 hover:bg-blue-700 text-white'
              }`}
            >
              {copied ? 'Copied!' : 'Copy'}
            </button>
          </div>
        </div>

        <div className="flex flex-col items-center">
          <label className="block text-sm font-medium text-gray-300 mb-3">QR Code</label>
          <div className="bg-white p-4 rounded-lg">
            <QRCodeSVG
              value={inviteUrl}
              size={180}
              level="M"
              includeMargin={false}
            />
          </div>
          <p className="text-gray-500 text-xs mt-2">Scan to open the registration page</p>
        </div>

        <div className="mt-6 pt-4 border-t border-gray-700">
          <div className="flex justify-between text-sm text-gray-400">
            <span>Expires:</span>
            <span>
              {invite.expires_at
                ? new Date(invite.expires_at).toLocaleDateString('en-US', {
                    year: 'numeric',
                    month: 'short',
                    day: 'numeric',
                    hour: '2-digit',
                    minute: '2-digit',
                  })
                : 'Never'}
            </span>
          </div>
        </div>

        <button
          onClick={onClose}
          className="w-full mt-4 px-4 py-2 bg-gray-700 hover:bg-gray-600 rounded-md font-medium transition-colors text-white"
        >
          Done
        </button>
      </div>
    </div>
  );
};

export default InviteLinkModal;
