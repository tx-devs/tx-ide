-- tx-client's nvim side, installed by `tx update` under <data>/nvim and put first on the
-- runtimepath of every nvim view tx-client starts. Activates the modules that work on their
-- own; tx-tour is required by whoever opens a tour. An operator config that already loads its
-- own copies (config.diffview-return, config.tx-ask) keeps them, so keymaps register once.
if not package.loaded["config.diffview-return"] then
  require("tx-diffview-return").setup()
end

if not (vim.g.tx_ask_loaded or package.loaded["config.tx-ask"]) then
  vim.g.tx_ask_loaded = true
  require("tx-ask")
end

-- The user's own layer, last: `<config>/nvim/tx.lua`, named by the view that started this nvim
-- (`g:tx_user_file`). It runs after everything above, so a mapping or autocmd it sets wins, and
-- it can delete the ones tx set.
local user_file = vim.g.tx_user_file
if user_file and vim.fn.filereadable(user_file) == 1 then
  dofile(user_file)
end
