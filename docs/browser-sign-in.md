# Browser sign-in

Recorde supports GitHub and GitLab device authorization with a registered public client ID. No client secret is bundled or accepted. A registration must exist on the exact provider host and allow device authorization. Bitbucket Cloud and Azure DevOps continue to use the token account form.

In Settings > Accounts & SSH, choose Add Account and select the provider and host. Enter the registered public client ID and choose Start browser sign-in. Copy the verification code, open the browser, and approve the request on the provider's page. Recorde verifies the returned identity before displaying **Save signed-in account**. That button stores credentials in the OS credential store. Cancel, changing provider/host, or closing the account view discards an unfinished or unsaved sign-in.

Settings contain only public registration metadata, keyed by exact host. Device codes, access tokens and refresh tokens never enter configuration files or browser URLs. HTTP redirects during authorization and refresh are refused. Polling respects the provider's minimum interval and increases it after a slow-down response. Cancellation and expiry stop further polling; a request already in flight has a bounded network timeout and cannot save credentials itself.

GitHub expiring tokens are requested with `offline_access`. Refresh requests rotate credentials under the credential store's process lock, check the account's stable identity, and verify the new vault value before returning the access token. Older enterprise installations may return non-expiring tokens. An expired token without usable refresh credentials requires another sign-in. See the [GitHub authorization and refresh documentation](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps).

The implementation currently uses device flow. GitHub recommends authorization code with PKCE for desktop applications, but its documented code exchange still requires a client secret. Recorde does not embed such a secret. A future PKCE flow needs a deliberately registered callback and a supported public-client exchange. See [GitHub OAuth security guidance](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/best-practices-for-creating-an-oauth-app).

GitLab must support device authorization, which became generally available in 17.9. OAuth tokens use Bearer authorization for API requests and `oauth2` as the Git HTTPS username. See [GitLab OAuth documentation](https://docs.gitlab.com/api/oauth2/). Existing personal access token accounts remain supported.

Bitbucket token accounts retain the Atlassian email address for API authentication. Embedded usernames in Bitbucket clone URLs are protocol hints; a repository account binding, or one unambiguous saved account, selects the email identity. Git HTTPS uses `x-bitbucket-api-token-auth`. Multiple saved accounts require explicit selection.

For matching saved provider accounts, GitHub's `x-access-token` and GitLab's `oauth2` clone URL usernames are also recognized as transport hints. Other embedded usernames must match the repository's selected account.

## Validation

Offline fixtures cover polling intervals, slow-down, cancellation, expiry, exact verification URLs, refused redirects, bounded/malformed responses, refreshed identities, token rotation and vault write verification. UI fixtures check cancellation/replacement of pending results and pointer activation of the explicit Save and Discard buttons. No fixture contacts an external provider or the real OS credential vault. A real sign-in requires a registered client ID and provider approval.
