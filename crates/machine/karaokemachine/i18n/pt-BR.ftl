# O que o editor de sincronia de letras diz na própria tela. Veja `src/sync.rs`.
#
# Os nomes das teclas (Space, Ctrl+S) ficam no código, pois são o que está impresso nelas.

## A janela e a linha no topo

sync-window-title = Sincronia de letra - { $title }
sync-mode-tapping = MARCANDO
sync-mode-review = REVISÃO
sync-playing = Tocando
sync-paused = Pausado
sync-status = { $transport }   { $position } / { $length }   andamento { $tempo }%   { $tapped } de { $total } marcadas
sync-status-unsaved = { $status }   não salvo
sync-channel = canal { $number }
sync-channel-named = canal { $number } ({ $name })
sync-vocal-label = Linha vocal: { $channel }
sync-vocal-label-silenced = Linha vocal: { $channel }, silenciada
sync-vocal-label-detecting = Linha vocal: detectando pelas suas marcas
sync-vocal-label-not-found = Linha vocal: nenhuma encontrada. M escolhe uma
sync-vocal-label-no-channel = Linha vocal: este arquivo não tem canal com notas afinadas
# A palavra selecionada dentro da linha, e onde ela começa.
sync-selected-at = { $line }   em { $time }

## Mensagens

sync-start = Aperte Enter para tocar a música, depois Space em cada palavra
# O editor foi aberto nas palavras do próprio arquivo, e num arquivo para continuar marcando.
sync-reopened = Estas são as palavras do arquivo, com o tempo delas. Mova uma palavra, depois Ctrl+S salva
sync-continued = { $tapped } palavras mantêm o tempo. Enter toca a partir da próxima palavra
sync-unsaved-close = Não salvo. Ctrl+S salva, fechar de novo sai
sync-unsaved-esc = Não salvo. Ctrl+S salva, Esc de novo sai
sync-saved = { $tapped } de { $total } palavras salvas em { $file }
sync-not-saved = Não salvo: { $reason }
sync-words-saved = As palavras foram salvas como texto em { $file }
sync-words-exist = { $file } já existe, então as palavras não foram gravadas
sync-nothing-to-review = Nada foi marcado ainda, então não há o que revisar
sync-tapping-again = Marcando de novo, a partir da próxima palavra
sync-cleared = Todas as marcações foram apagadas. Aperte Enter para tocar, ou Ctrl+Shift+Backspace de novo para recuperá-las
sync-clear-undone = As marcações voltaram
sync-review-so-far = Revisão das { $tapped } palavras marcadas até aqui. R volta a marcar
sync-all-tapped = Todas as palavras estão marcadas. Pressione R para revisar, ou Backspace para desfazer a última marca
sync-review-all = Revisão. A música repete com o seu tempo, para conferir. R volta a marcar
sync-all-tapped-no-vocal = Todas as palavras marcadas, e suas marcas não seguem nenhum canal. M escolhe a linha vocal
sync-paused-tap = A música está pausada. Enter toca
sync-word-ends = "{ $word }" termina aqui
sync-end-refused = E termina uma palavra depois que ela começa, com a música tocando
sync-vocal-is = A linha vocal é o { $channel }. V silencia, para ouvir o que sobra
sync-vocal-is-silenced = A linha vocal é o { $channel }, e está silenciada
sync-no-vocal = Nenhuma linha vocal escolhida. M escolhe o canal
sync-silenced = O { $channel } está silenciado. V traz de volta
sync-sounds-again = O { $channel } voltou a soar
sync-taps-follow = Suas marcas seguem o { $channel }, então ele é a linha vocal. M muda
sync-snapped = { $moved } palavras movidas para as notas do { $channel }. Ctrl+Z desfaz
sync-snap-undone = As palavras voltaram para onde você marcou
sync-no-audio = Sem saída de áudio. Tentando de novo
sync-audio-back = A saída de áudio voltou

## A lista de teclas: sobre o que as teclas da linha agem, depois o que cada uma faz

sync-row-tap = MARCAR
sync-row-word = PALAVRA
sync-row-song = MÚSICA
sync-row-notes = NOTAS
sync-row-file = ARQUIVO
sync-key-next-word = próxima palavra
sync-key-end-word = terminar a palavra
sync-key-undo = desfazer
sync-key-tap-line-again = marcar esta linha de novo
sync-key-play-pause = tocar / pausar
sync-key-seek = 5 s
sync-key-tempo = mais lento / mais rápido
sync-key-vocal-next = próximo canal como linha vocal (Shift: anterior)
sync-key-silence = silenciar
sync-key-save = salvar
sync-key-save-words = salvar as palavras em um .txt, se não houver
sync-key-leave = sair
sync-key-hide-keys = esconder estas teclas
sync-key-show-keys = teclas
sync-key-review-so-far = revisar o que já foi marcado
sync-key-back-to-tapping = voltar a marcar
sync-key-clear = apagar todas as marcações
sync-key-select = selecionar
sync-key-select-sung = selecionar a palavra cantada
sync-key-end-here = terminar aqui
sync-key-move = mover 10 ms (Shift 50)
sync-key-play-line = tocar esta linha
sync-key-snap = mover as palavras para as notas dela (Ctrl+Z desfaz)
