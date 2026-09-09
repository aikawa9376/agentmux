-- Executed only on the Neovim socket published by the ACP owner.
(function(request)
  if vim.fn.getpid() ~= request.pid then return 'unavailable' end
  local ok, state = pcall(require, 'lazyagent.logic.state')
  if not ok then return 'unavailable' end
  local selected, name
  for key, session in pairs(state.sessions or {}) do
    if session.backend == 'buffer_acp'
      and (session.acp_transcript_path or session.transcript_path) == request.preview then
      if selected then return 'unavailable' end
      selected, name = session, key
    end
  end
  if not selected then return 'unavailable' end
  local _, backend = require('lazyagent.logic.backend').resolve_backend_for_agent(name, nil)
  if not backend then return 'unavailable' end
  local accepted
  if request.action == 'send' and type(backend.paste_and_submit) == 'function' then
    accepted = backend.paste_and_submit(selected.pane_id, request.text, { 'C-m' }, {})
  elseif request.action == 'interrupt' and type(backend.send_keys) == 'function' then
    accepted = backend.send_keys(selected.pane_id, 'C-c')
  else return 'unavailable' end
  return accepted == false and 'rejected' or 'accepted'
end)(_A)
