call plug#begin('~/nvim/plugged')
Plug 'tpope/vim-fugitive'
Plug 'tpope/vim-commentary'
Plug 'ludovicchabant/vim-gutentags'
Plug 'preservim/nerdtree'
Plug 'Xuyuanp/nerdtree-git-plugin'
Plug 'itchyny/lightline.vim'
Plug 'StanAngeloff/php.vim'
Plug 'stephpy/vim-php-cs-fixer'
Plug 'junegunn/fzf.vim'
Plug 'sheerun/vim-polyglot'
Plug 'mhinz/vim-signify'
Plug 'phpactor/phpactor', {'for': 'php', 'do': 'composer install'}
Plug 'ncm2/ncm2'
Plug 'roxma/nvim-yarp'
Plug 'ncm2/ncm2-bufword'
Plug 'ncm2/ncm2-path'
Plug 'ncm2/ncm2-github'
call plug#end()

" ============================================================================
" => General settings

let g:python3_host_prog = expand('$HOME') . '/.local/bin/python3'

set number 
set relativenumber

let g:mapleader=","

" ============================================================================
" => ludovicchabant/vim-gutentags

" update tags in background whenever you write a php file
au BufWritePost *.php silent! !eval '[ -f ".git/hooks/ctags" ] && .git/hooks/ctags' &

set statusline+=%{gutentags#statusline()}

augroup MyGutentagsStatusLineRefresher
    autocmd!
    autocmd User GutentagsUpdating call lightline#update()
    autocmd User GutentagsUpdated call lightline#update()
augroup END

" ============================================================================
" => ncm2/ncm2

" enable ncm2 for all buffers
autocmd BufEnter * call ncm2#enable_for_buffer()

" IMPORTANT: :help Ncm2PopupOpen for more information
set completeopt=noinsert,menuone,noselect

" Use <TAB> to select the popup menu:
inoremap <expr> <Tab> pumvisible() ? "\<C-n>" : "\<Tab>"
inoremap <expr> <S-Tab> pumvisible() ? "\<C-p>" : "\<S-Tab>"

" ============================================================================
" => phpactor/phpactor

" Include use statement
nmap <Leader>uu :call phpactor#UseAdd()<CR>

" Find references
nmap <Leader>ff :call phpactor#FindReferences()<CR>

" Invoke the context menu
nmap <Leader>mm :call phpactor#ContextMenu()<CR>

" Invoke the navigation menu
nmap <Leader>nn :call phpactor#Navigate()<CR>

" Goto definition of class or class member under the cursor
nmap <Leader>gg :call phpactor#GotoDefinition()<CR>
nmap <Leader>gt :call phpactor#GotoDefinitionTab()<CR>

" Show brief information about the symbol under the cursor
nmap <Leader>hh :call phpactor#Hover()<CR>

" Transform the classes in the current file
nmap <Leader>tt :call phpactor#Transform()<CR>

" Generate a new class (replacing the current file)
nmap <Leader>cc :call phpactor#ClassNew()<CR>

" Extract expression (normal mode)
nmap <silent><Leader>ee :call phpactor#ExtractExpression(v:false)<CR>

" Extract expression from selection
vmap <silent><Leader>ee :<C-U>call phpactor#ExtractExpression(v:true)<CR>

" Extract method from selection
vmap <silent><Leader>em :<C-U>call phpactor#ExtractMethod()<CR>

let g:phpactorOmniAutoClassImport = v:true
let g:phpactorInputListStrategy = 'fzf'

" ============================================================================
" => preservim/nerdtree

let NERDTreeShowHidden=1
let NERDTreeMapOpenInTab='<ENTER>'

map <C-n> :NERDTreeToggle<CR>

" ============================================================================
" => stephpy/vim-php-cs-fixer

let g:php_cs_fixer_rules = "@PSR2"                " options: --rules (default:@PSR2)

let g:php_cs_fixer_php_path = "php"               " Path to PHP
let g:php_cs_fixer_enable_default_mapping = 1     " Enable the mapping by default (<leader>pcd)
let g:php_cs_fixer_dry_run = 0                    " Call command with dry-run option
let g:php_cs_fixer_verbose = 0                    " Return the output of command if 1, else an inline information.

nnoremap <silent><leader>pcd :call PhpCsFixerFixDirectory()<CR>
nnoremap <silent><leader>pcf :call PhpCsFixerFixFile()<CR>

" Runs the tool automatically 
autocmd BufWritePost *.php silent! call PhpCsFixerFixFile()

" ============================================================================
" => junegunn/fzf.vim

" If installed using Homebrew
set rtp+=/usr/local/opt/fzf

" ctrl-p to open a file via fzf
nnoremap <C-p> :FZF!<cr>

nnoremap <leader>a :Rg<space>
nnoremap <leader>A :exec "Rg ".expand("<cword>")<cr>

autocmd VimEnter * command! -nargs=* Rg
  \ call fzf#vim#grep(
  \   'rg --column --line-number --no-heading --fixed-strings --ignore-case --no-ignore --hidden --follow --glob "!.git/*" --color "always" '.shellescape(<q-args>), 1,
  \   <bang>0 ? fzf#vim#with_preview('up:60%')
  \           : fzf#vim#with_preview('right:50%:hidden', '?'),
  \   <bang>0)

" ============================================================================
" => mhinz/vim-signify

" default updatetime 4000ms is not good for async update
set updatetime=100

" ============================================================================
" => sheerun/vim-polyglot

let g:polyglot_disabled = ['php']
