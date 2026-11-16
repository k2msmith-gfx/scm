" Each op: the keys fed and executed, then a forced redraw (the terminal
" write), timed together. Insert mode cannot be driven synchronously
" (feedkeys 'x!' waits in Insert mode for more input), so typing is driven
" from a timer: a key fed as typed, and the next tick — which runs once
" Neovim has processed it and is waiting again — redraws and takes the time.
let g:out = get(g:, 'bench_out', 'vim.txt')
call delete(g:out)

func! s:write(name, times)
  call writefile([a:name . ' ' . join(map(copy(a:times), 'printf("%.6f", v:val)'), ' ')], g:out, 'a')
endfunc

func! s:run(name, keys_list, flags)
  let times = []
  for k in a:keys_list
    let t = reltime()
    call feedkeys(k, a:flags)
    redraw
    call add(times, reltimefloat(reltime(t)))
  endfor
  call s:write(a:name, times)
endfunc

func! s:bench(timer)
  " 1. scrolling: j from the top, then PageDown
  call feedkeys("gg", 'xt') | redraw
  call s:run('scroll-j', repeat(['j'], 1000), 'xt')
  call s:run('page-down', repeat(["\<PageDown>"], 100), 'xt')
  " 2. inserting: a new line at 3000, 600 characters typed
  let s:chars = []
  for i in range(600)
    call add(s:chars, (i % 60 == 59) ? "\<CR>" : (i % 7 == 6 ? ' ' : nr2char(97 + i % 26)))
  endfor
  let s:i = 0
  let s:times = []
  call feedkeys("3000Go", 'nt')
  call timer_start(200, function('s:type'))
endfunc

func! s:type(timer)
  if s:i > 0
    redraw
    call add(s:times, reltimefloat(reltime(s:t0)))
  endif
  if s:i >= len(s:chars)
    call s:write('insert', s:times)
    call feedkeys("\<Esc>gg", 'nt')
    call timer_start(200, function('s:search'))
    return
  endif
  let s:t0 = reltime()
  call feedkeys(s:chars[s:i], 'nt')
  let s:i += 1
  call timer_start(0, function('s:type'))
endfunc

func! s:search(timer)
  redraw
  call s:run('search', repeat(["/define\<CR>"], 200), 'xt')
  call writefile(['lines ' . &lines . ' columns ' . &columns . ' cursor ' . line('.') . ':' . col('.') . ' mode ' . mode()], g:out, 'a')
  call writefile(getline(1, '$'), g:out . '.text')
  qa!
endfunc

" Let the window settle, the highlighter attach and any LSP start.
call timer_start(3000, function('s:bench'))
