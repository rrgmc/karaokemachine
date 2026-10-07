# km-song-sync: tudo o que a página diz, em português do Brasil.
#
# Uma mensagem com `{ $variável }` é preenchida pelo Rust. Um template só nomeia mensagens sem
# nenhuma.

app-title = KaraokeMachine Song Sync
language-picker = Idioma
action-quit = Sair

home-heading = Colocar a letra em um arquivo MIDI
home-intro = Encontre o arquivo MIDI de uma música e pressione Selecionar. Cole a letra na caixa abaixo e pressione Iniciar. O editor de sincronia abre, e você marca cada sílaba no tempo da música. O editor salva um novo arquivo .kar ao lado da música e nunca altera a música.
home-no-machine = A máquina de karaokê não foi encontrada ao lado deste programa, então nada pode ser iniciado. Instale a máquina, ou indique-a com --machine-exe.

words-label = A letra
words-placeholder = Um verso da música em cada linha. Deixe uma linha vazia entre as estrofes.
words-hint = Um hífen divide a palavra em sílabas: ka-ra-o-ke são quatro toques. Digite \- para um hífen que é cantado. Sem a marca, a música usa o arquivo de texto ao lado dela, ou a letra que ela já tem.
words-continue = A música já está marcada em parte: manter essas palavras como estão, e marcar só o resto
words-use = Usar a letra desta caixa
words-selected = Música:
words-selected-none = Nenhuma selecionada. Pressione Selecionar em uma música acima.
names-title = Título
names-artist = Artista
names-language = Idioma
names-language-none = Sem alteração
names-hint = Estes dados vão para o arquivo sincronizado. Corrija o que a música informa errado. Um campo vazio mantém o que a música informa, e uma música sem título recebe o nome do arquivo.
column-file = Arquivo
column-title = Título
column-artist = Artista
action-clear = Limpar

action-start = Iniciar
action-select = Selecionar
action-reveal = Mostrar na pasta
action-go = Ir

browse-top-title = Ir para o topo
browse-up-title = Subir uma pasta
browse-this-computer = Este computador
browse-folder-label = O caminho completo de uma pasta
browse-named = Com o nome
browse-part-of-a-name = Parte de um nome
browse-nothing-found = Não há pastas nem arquivos MIDI aqui.
browse-nothing-called-that = Nada aqui tem isso no nome.
browse-range = { $first } a { $last } de { $count }
page-previous = Anterior
page-next = Próxima

row-not-midi = Este não é um arquivo MIDI que o editor consegue ler.
row-uses-text-file = Usa a letra de { $file }, a menos que a caixa abaixo esteja marcada.
row-own-words = Já tem letra. O editor a abre para correção, a menos que a caixa abaixo esteja marcada.
row-needs-words = Não tem letra. Cole-a na caixa abaixo.
row-output-exists = { $file } já está aqui.
words-replace = Substituir o arquivo sincronizado que já existe

editor-running = O editor está aberto em { $song }.
editor-running-hint = Pressione Ctrl+S no editor para salvar, e feche a janela dele quando terminar.
editor-saved = { $file } foi salvo.
editor-saved-nothing = O editor fechou em { $song } e não salvou nada.
editor-failed = O editor não abriu { $song }.

said-busy = O editor já está aberto. Feche-o antes de iniciar outra música.
said-no-song = Selecione um arquivo MIDI primeiro.
said-no-machine = A máquina de karaokê não foi encontrada, então o editor não pode iniciar.
said-no-words = Essa música não tem letra. Cole-a na caixa e marque-a.
said-box-empty = A caixa da letra está vazia. Cole a letra, ou tire a marca.
said-not-midi = Esse não é um arquivo MIDI que o editor consegue ler.
said-output-exists = O arquivo sincronizado já existe. Marque a caixa que o substitui primeiro.
said-reveal-failed = Não foi possível abrir a pasta.
