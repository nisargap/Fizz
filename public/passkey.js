// Passkeys through Fizzlayer's /api/auth. Supabase's options and the browser's responses use
// WebAuthn's JSON form, so they pass straight through.
window.fizzPasskey = {
  supported: Boolean(window.PublicKeyCredential?.parseRequestOptionsFromJSON && navigator.credentials),
  async credential(options, creating) {
    const publicKey = options.publicKey || options;
    const credential = creating
      ? await navigator.credentials.create({ publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(publicKey) })
      : await navigator.credentials.get({ publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(publicKey) });
    return credential.toJSON();
  },
  // The browser throws NotAllowedError when the person closes the passkey prompt.
  cancelled: (error) => error?.name === 'NotAllowedError' || error?.name === 'AbortError',
};
