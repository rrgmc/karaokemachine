# As páginas do KaraokeMachine Package Builder, em português do Brasil.
#
# **Linguagem simples de aplicativo**, a regra com que todo catálogo do projeto começa. Quem lê aqui
# é quem organiza um acervo de centenas de milhares de arquivos em um computador, então o registro é
# o de quem opera: uma ou duas frases, a consequência primeiro, e uma palavra para o controle em vez
# de uma explicação dele.
#
# **Esta é uma tradução, e o português é quem a governa.** Não é o inglês palavra por palavra: a
# frase é escrita como se escreve em português. Veja `Translations are governed by their own
# language`.
#
# **`locale` é a língua desta interface e `language` é a de uma música.** As duas aparecem na mesma
# página aqui, então uma chave diz `settings-locale-` para a primeira e `songs-language-` para a
# segunda, e nenhuma das duas atravessa.
#
# **Os dados ficam como chegam.** O título de uma música, o artista, o caminho de uma pasta, uma
# etiqueta que alguém digitou e os nomes que `km_kmpkg::Language` carrega para os seletores de língua
# são valores, e traduzi-los não é tarefa deste arquivo.

## O cabeçalho ---------------------------------------------------------------

nav-songs = Músicas
nav-lyrics = Letras
nav-folders = Pastas
nav-favorites = Favoritas
nav-duplicates = Repetidas
nav-packages = Pacotes
nav-scan = Leitura
nav-settings = Ajustes

nav-lyrics-title = procurar uma música por um trecho da letra
nav-root-title = abrir outra pasta

header-songs = { $count ->
    [0] { $count } músicas
    [one] { $count } música
   *[other] { $count } músicas
  }
header-files = { $count ->
    [0] { $count } arquivos
    [one] { $count } arquivo
   *[other] { $count } arquivos
  }
header-failed = { $count ->
    [0] { $count } com falha
    [one] { $count } com falha
   *[other] { $count } com falha
  }
header-favorites = { $count ->
    [0] { $count } favoritas
    [one] { $count } favorita
   *[other] { $count } favoritas
  }

header-machine-title = { $address } — mude em Ajustes
header-machine-none = nenhuma máquina definida
header-machine-none-title = Tocar e Instalar não têm para onde ir enquanto isto não for definido
header-version-title = a versão deste programa

header-open-browser = Abrir no navegador
header-open-browser-title = abrir esta página no seu navegador de sempre
header-quit = Sair
header-quit-title = parar o programa

## Palavras que um controle diz em mais de uma página --------------------------

action-save = Salvar
action-cancel = Cancelar
action-ok = Ok

## Agir sobre um filtro inteiro ------------------------------------------------
#
# As cinco confirmações dizem as mesmas três coisas: quantas, quais músicas e o que aconteceria com
# elas. A contagem e o botão são compostos no Rust, e cada língua escolhe a própria forma do plural.

confirm-songs = { $count ->
    [0] { $count } músicas
    [one] { $count } música
   *[other] { $count } músicas
  }
confirm-files = { $count ->
    [0] { $count } arquivos
    [one] { $count } arquivo
   *[other] { $count } arquivos
  }
confirm-whole-corpus = o acervo inteiro, porque nenhum filtro está posto
confirm-whole-corpus-unnarrowed = o acervo inteiro, porque nenhum filtro está limitando a lista
confirm-matching = que correspondem a
confirm-with = com
confirm-without = sem
confirm-into = para dentro de
confirm-out-of = para fora de
confirm-read-again = ler de novo

confirm-set = Sim, definir { $count }
confirm-tag = Sim, etiquetar { $count }
confirm-untag = Sim, tirar a etiqueta de { $count }
confirm-file = Sim, guardar { $count }
confirm-unfile = Sim, tirar { $count }
confirm-reread = Sim, reler { $count }
package-add-room-left = que tem lugar para { $room }, então as { $room } primeiras desta ordem entram

## Filtros que alguém nomeou ----------------------------------------------------

saved-band = salvos
saved-none = Nada salvo ainda — limite a lista e depois dê um nome a ela.
saved-name-placeholder = dê um nome a este filtro
saved-keep-page = manter a página
saved-whole-corpus = o acervo inteiro
saved-update-title = fazer { $name } passar a significar o filtro que está na tela agora
saved-rename-title = renomear este
saved-forget-title = esquecer este
saved-forget-confirm = Esquecer o filtro salvo “{ $name }”? Nenhuma música é alterada.
saved-already-saved = já está salvo, como
saved-replace-it = Sim, substituir

## As páginas da lista -----------------------------------------------------------

songs-range = página { $page } de { $pages } ({ $total ->
    [one] { $total } música
   *[other] { $total } músicas
  })
songs-range-scanning = página { $page } de cerca de { $pages } (cerca de { $total ->
    [one] { $total } música
   *[other] { $total } músicas
  })
pager-scanning = lendo
pager-scanning-title = uma leitura ainda está gravando linhas.
pager-first-title = a primeira página
pager-last-title = a última página

## A letra de uma música ---------------------------------------------------------

lyrics-decoded-as = lida como
lyrics-pin-encoding = Fixar esta codificação
lyrics-none = Este arquivo não tem letra.

## Abrir uma pasta ----------------------------------------------------------------

open-top-title = o topo
open-this-computer = este computador
open-this-folder = Abrir esta pasta
open-create-database = Criar um banco de dados aqui
open-folders-named = pastas chamadas
open-part-of-a-name = parte de um nome

## As etiquetas da barra de filtros --------------------------------------------

chips-showing = mostrando
chips-remove-title = parar de filtrar por isto
chips-clear-all = limpar tudo

## As etiquetas de uma música ---------------------------------------------------

song-tags-none = Nenhuma.
song-tag-remove-title = tirar esta etiqueta

## Os eventos de texto de uma música --------------------------------------------
#
# A aba com o que o arquivo guarda. As três primeiras colunas usam as abreviações que as telas de um
# sequenciador usam.

raw-none = Nenhum evento de texto.
raw-track = Trilha
raw-tick = Tick
raw-kind = Tipo
raw-text = Texto

## Páginas ----------------------------------------------------------------------

pager-previous = anteriores
pager-next = próximas
hits-range = página { $page } de { $pages } ({ $total ->
    [one] { $total } música
   *[other] { $total } músicas
  }), as mais parecidas primeiro

## Os dois botões de um pacote --------------------------------------------------

install-button = Instalar na máquina atual
install-build-first = Monte o pacote primeiro.
install-sends = Envia o pacote montado para a máquina de karaokê.
renumber-button = Renumerar tudo a partir de { $start }
renumber-note = Mantém a ordem atual. Qualquer número que você tenha posto à mão é substituído.

## Procurar uma máquina na rede -------------------------------------------------

discovered-none-lead = Nenhuma máquina foi encontrada nesta rede. Uma máquina só aparece enquanto
discovered-none-tail = estiver ligado.
discovered-select = Usar
discovered-already-set = já é a atual

## Ajustes: a língua deste programa -------------------------------------------

settings-locale-heading = A língua deste programa
settings-locale-field = Mostrar estas páginas em
settings-locale-note = A língua das páginas do próprio programa. Não muda nada nas músicas: a língua de uma música é aquela em que ela é cantada, e você define isso na música ou no pacote.
settings-locale-save = Salvar

## O que o navegador diz por conta própria ------------------------------------

js-answered = { $what } respondeu { $status }
js-unreachable = Não foi possível falar com o package builder. Ele parou?
js-timed-out = { $what } demorou demais e foi abandonado.
js-swap-failed = Não foi possível desenhar a resposta de { $what }.
js-the-tool = o programa

## Arquivos que não foram lidos --------------------------------------------------

failures-heading = Arquivos que não foram lidos
failures-count = Quantos
failures-reason = Motivo
failures-example = Exemplo
failures-remove = Tirar da lista
failures-restore = Trazer de volta
failures-remove-title = { $count ->
    [0] Aceita os { $count } arquivos que falham assim agora. Um arquivo que passe a falhar assim depois volta para a lista.
    [one] Aceita o { $count } arquivo que falha assim agora. Um arquivo que passe a falhar assim depois volta para a lista.
   *[other] Aceita os { $count } arquivos que falham assim agora. Um arquivo que passe a falhar assim depois volta para a lista.
  }
failures-reasons-removed = { $count ->
    [0] { $count } motivos fora da lista
    [one] { $count } motivo fora da lista
   *[other] { $count } motivos fora da lista
  }
failures-still-here = Estes arquivos continuam aqui e continuam sem ser lidos. Tirar um motivo só o retira da lista acima.

failure-parsed = lido
failure-unreadable = não foi possível ler
failure-not-midi = não é um arquivo MIDI legível
failure-not-video = não é um arquivo de vídeo legível
failure-video-unsupported = é um vídeo, e esta compilação não tem o recurso `video` para lê-lo
failure-missing-graphics = é um MP3 sem .cdg ao lado, então não tem letra
failure-orphan-graphics = é um .cdg sem áudio ao lado
failure-not-audio = não é um arquivo de áudio legível
failure-bad-graphics = é um .cdg que não desenha letra nenhuma
failure-bad-ultrastar = é um arquivo UltraStar que esta máquina não toca
failure-ultrastar-audio = é um arquivo UltraStar cujo MP3 não está ao lado
failure-panicked = o leitor quebrou

## Ler o acervo --------------------------------------------------------------------

scan-reads-lead = Lê e interpreta todo arquivo compatível dentro de
scan-unchanged-skipped = Arquivos são pulados quando nada neles mudou e esta versão do programa os analisaria da mesma forma.
scan-reanalyze = Reler tudo
scan-reanalyze-note = lê todos de novo e termina reagrupando os arquivos que parecem uma mesma gravação.
scan-changed = Ler o que mudou
scan-last-run = última vez
scan-never-run = nunca feito nesta pasta
scan-stale-analysis = { $count ->
    [0] { $count } músicas foram analisadas por uma versão anterior deste programa. Ler a pasta atualiza elas; não há mais nada a fazer.
    [one] { $count } música foi analisada por uma versão anterior deste programa. Ler a pasta atualiza ela; não há mais nada a fazer.
   *[other] { $count } músicas foram analisadas por uma versão anterior deste programa. Ler a pasta atualiza elas; não há mais nada a fazer.
  }
scan-browse-what-was-found = Ver o que foi encontrado
scan-stopped-early = Parou antes de terminar. Tudo o que foi lido até aqui está salvo, mas a pasta foi lida só em parte e não conta como lida. Mande ler de novo para continuar: os arquivos que não mudaram são pulados, então a leitura retoma de onde parou.

scan-tally = { $done } de { $total } lidos · { $percent }% no banco de dados ({ $written } gravados nesta vez) · { $parsed } analisados · { $skipped } sem mudança
scan-failed = { $count ->
    [0] { $count } com falha
    [one] { $count } com falha
   *[other] { $count } com falha
  }
scan-waiting = { $count ->
    [0] { $count } esperando para serem gravados
    [one] { $count } esperando para ser gravado
   *[other] { $count } esperando para serem gravados
  }
scan-waiting-title = lidos e analisados, ainda não gravados. Os leitores correm à frente da única linha de gravação, então a navegação vai continuar achando músicas novas até isto chegar a zero.

scan-found = { $count ->
    [0] { $count } arquivos encontrados até agora
    [one] { $count } arquivo encontrado até agora
   *[other] { $count } arquivos encontrados até agora
  }
scan-rate = { $rate } arquivos por segundo
scan-remaining = faltam uns { $time }
scan-steps = Etapas
scan-step-if-changed = só se algo mudou
scan-step-not-needed = não foi preciso desta vez
scan-step-not-reached = não chegou aqui
scan-stop = Parar
scan-stop-title = Para depois de gravar os arquivos já lidos. Tudo o que foi lido fica salvo, e a próxima leitura continua de onde parou.
scan-stopping = Parando: gravando os arquivos já lidos.

scan-meter-reading = lendo
scan-phase-preparing = carregando o que a última leitura encontrou
scan-phase-looking = procurando arquivos
scan-phase-reading = lendo e analisando
scan-phase-indexing = indexando as pastas
scan-phase-forgetting = esquecendo arquivos que sumiram
scan-phase-duplicates = procurando repetições
scan-phase-measuring = medindo o acervo para o planejador de consultas
scan-phase-stopped = parado
scan-phase-finished = terminado

## As colunas da lista ---------------------------------------------------------------

songs-no-matches = Nada corresponde.
songs-select-all-title = marcar todas desta página; com shift, clique em duas caixas para marcar as linhas entre elas
column-artist = Artista
column-title = Título
column-language = Língua
column-length = Dur.
column-suitability = Adequação
column-score = Nota
column-score-title = A sua nota
column-melody = Melodia
column-melody-title = Marcado quando um canal de melodia foi detectado; passe o mouse para ver o canal
column-copies = Cópias
column-copies-title = Cópias idênticas byte a byte no disco
column-versions = Versões
column-versions-title = Arquivos que parecem a mesma gravação, salvos de outro jeito

## Andar pelas pastas deste computador ------------------------------------------------

open-indexed = indexada
open-open = Abrir
open-nothing-found = nada encontrado
open-no-folder-called-that = nenhuma pasta aqui se chama assim
folders-range = página { $page } de { $pages } ({ $total ->
    [one] { $total } pasta
   *[other] { $total } pastas
  })

## Favoritas -----------------------------------------------------------------------

favorites-new-placeholder = nova lista
favorites-create = Criar
favorites-none-yet = Nenhuma ainda.
favorites-name = Nome
favorites-songs = Músicas
favorites-second-copies = Cópias repetidas
favorites-second-copies-title = Entradas que são uma segunda versão de uma música que já está nesta lista
favorites-working-list = Lista de trabalho
favorites-working-list-title = Uma lista que é rascunho para uma passagem depois, e não um arquivamento
favorites-set-aside-title = as músicas desta lista foram separadas, não arquivadas
favorites-rename = Renomear
favorites-tidy = Limpar
favorites-delete = Excluir
favorites-tidy-confirm = { $count ->
    [0] Tirar { $count } entradas de { $name }? Cada uma é um segundo arquivo de uma música que a lista guarda; a melhor cópia de cada uma continua.
    [one] Tirar { $count } entrada de { $name }? Ela é um segundo arquivo de uma música que a lista guarda; a melhor cópia de cada uma continua.
   *[other] Tirar { $count } entradas de { $name }? Cada uma é um segundo arquivo de uma música que a lista guarda; a melhor cópia de cada uma continua.
  }
favorites-delete-confirm = Excluir { $name }? As músicas em si não são alteradas.
favorites-delete-confirm-sourcing = { $count ->
    [0] Estes pacotes têm o que esta lista tem, e a próxima sincronização deles tiraria toda música que ela pôs lá: { $packages }.
    [one] { $packages } tem o que esta lista tem, e a próxima sincronização dele tiraria toda música que esta lista pôs lá.
   *[other] Estes pacotes têm o que esta lista tem, e a próxima sincronização deles tiraria toda música que ela pôs lá: { $packages }.
  }

## Procurar pela letra ---------------------------------------------------------------

lyrics-contain = a letra tem
lyrics-part-of-a-lyric = um trecho da letra
lyrics-how-it-matches = Os acentos são ignorados. A última palavra vale como começo de palavra. Aspas pedem as palavras nessa ordem.

## As duas abas de curadoria ---------------------------------------------------------

curate-package = Pacote
curate-add = Adicionar
curate-no-packages = Nenhum pacote ainda.
curate-put-ticked-in = guardar as marcadas em
curate-file = Guardar
curate-no-favorites = Nenhuma lista ainda.

## Montar um pacote ---------------------------------------------------------------

build-phase-starting = começando
build-phase-reading = lendo o pacote
build-phase-packaging = { $count ->
    [0] empacotando { $count } músicas
    [one] empacotando { $count } música
   *[other] empacotando { $count } músicas
  }
build-phase-song = música { $index } de { $total } — { $name }
build-phase-encoding = recodificando a música { $number } — esta demora
build-phase-writing = gravando { $name } — não há mais músicas para ler
build-phase-done = pronto
build-phase-stopped = parado

build-done = { $done } de { $total } ({ $percent }%)
build-volume-now = Volume { $number }
build-written = { $count } dentro
build-skipped = { $count } de fora
build-encoding = recodificando esta, { $percent }% — a contagem de músicas não anda enquanto isso

## Procurar pela letra, continuação --------------------------------------------------

lyrics-type-a-line = Digite um trecho e as músicas que o cantam aparecem aqui.
lyrics-none-indexed-lead = Nenhuma letra foi indexada ainda. Mande ler com “reler todo arquivo” marcado na página
lyrics-none-indexed-tail = .
lyrics-not-found = Nada encontrado.

## Músicas com nome parecido ---------------------------------------------------------

similar-heading = Músicas com nome parecido
similar-how-it-matches = Maiúsculas, acentos, pontuação e palavras como “the” ou “karaoke” são ignorados. Uma palavra com erro de digitação ainda é encontrada. Os nomes mais parecidos vêm primeiro, e os últimos podem ser outras músicas.
similar-type-a-name = Digite um título ou um artista e as músicas com nome parecido aparecem aqui.
similar-not-found = Nenhuma música tem nome parecido.
similar-likeness-title = o quanto os dois nomes se parecem
similar-searched-from-title = a música de onde esta busca partiu
column-likeness = Semelhança
column-language-title = A língua em que é cantada, como código ISO 639-1
column-suitability-title = Adequação automática, de 0 a 10
column-score-own-title = A sua nota, de 0 a 10 ou em branco

## A árvore de pastas -----------------------------------------------------------------

folders-all = todas as pastas
folders-none-lead = Nenhuma pasta para mostrar. Ou esta pasta não tem músicas legíveis, ou o acervo ainda não foi
folders-none-link = lido
folders-none-tail = .
folders-folder = Pasta
folders-songs-title = Músicas distintas em qualquer lugar abaixo dela, não arquivos
folders-up = subir
folders-files-here = arquivos aqui
folders-browse = Abrir
folders-only-this = Só esta pasta

## Uma linha da lista ---------------------------------------------------------------

row-artist-title = todas as músicas de { $artist }
row-melody-title = canal { $channel }
row-versions-title = { $count ->
    [0] esta linha vale por { $count } arquivos que parecem uma mesma gravação
    [one] esta linha vale por { $count } arquivo que parece uma mesma gravação
   *[other] esta linha vale por { $count } arquivos que parecem uma mesma gravação
  }
row-hidden-version-count-title = { $count ->
    [0] um de { $count } arquivos que parecem uma mesma gravação
    [one] um de { $count } arquivo que parece uma mesma gravação
   *[other] um de { $count } arquivos que parecem uma mesma gravação
  }
row-hidden-version = oculta
row-hidden-version-title = a lista de músicas mostra outra versão desta gravação no lugar desta; abrir essa
row-favorites-close = fechar a escolha
row-favorites-filed = { $count ->
    [0] está em { $count } listas — escolha outra, ou tire de uma
    [one] está em { $count } lista — escolha outra, ou tire de uma
   *[other] está em { $count } listas — escolha outra, ou tire de uma
  }
row-favorites-working = { $count ->
    [0] está em { $count } listas de trabalho e em nenhuma arquivada — escolha uma lista, ou tire de uma
    [one] está em { $count } lista de trabalho e em nenhuma arquivada — escolha uma lista, ou tire de uma
   *[other] está em { $count } listas de trabalho e em nenhuma arquivada — escolha uma lista, ou tire de uma
  }
row-favorites-none = guardar numa lista
row-take-out-of = tirar de { $name }
row-put-in = pôr em { $name }

row-from-file-name = nome do arquivo
row-no-title-title = nenhum título foi encontrado dentro do arquivo
row-edited = ed
row-edited-title = editado
row-file-name-title = nome do arquivo
row-language-unknown = sem língua
row-language-more = mais
row-set = Definir
row-score-title = a sua nota
row-edit-title = editar o título e o artista aqui
row-youtube-title = procurar no YouTube
row-similar-title = procurar músicas com nome parecido
row-which-favorite = em qual lista?
row-working-lists = listas de trabalho
row-no-favorites = Nenhuma ainda — dê um nome e ela é criada e preenchida de uma vez.
row-create-and-file = Criar e guardar

## Abrir uma pasta, enquanto demora ---------------------------------------------------

opening-database = abrindo o banco de dados
opening-closing-previous = fechando a pasta que estava aberta
opening-up-to-date = atualizando o banco de dados
opening-indexing = { $missing ->
    [0] criando { $missing } índices que faltam e reunindo estatísticas
    [one] criando { $missing } índice que falta e reunindo estatísticas
   *[other] criando { $missing } índices que faltam e reunindo estatísticas
  }
opening-working-out-language = descobrindo a língua de cada música
opening-tidying-text = arrumando o texto lido dos arquivos
opening-folding = preparando { $done } de { $total } títulos para a ordem da lista
opening-gathering-statistics = reunindo estatísticas do acervo inteiro
opening-folding-journal = juntando o diário de volta ao banco de dados
opening-finishing = terminando

## O seletor de pastas --------------------------------------------------------------

open-title = Abrir uma pasta
open-back = voltar
open-opening-a-folder = abrindo uma pasta
open-choose-a-folder = escolha uma pasta de arquivos de karaokê
open-opening = Abrindo
open-opened = Aberta
open-progress = Abrindo { $root } — { $phase } · { $seconds }s.
open-migrating-hint = Um acervo grande é migrado na primeira vez que uma nova versão o abre, e isso pode levar minutos. Se você parar, ele retoma depois.
open-recent = Recentes
open-database-gone = o banco de dados de curadoria sumiu
open-not-found = não encontrada
open-forget-title = tirar desta lista
open-browse = Procurar
open-browse-this-computer = Procurar neste computador
open-or-type-a-path = Ou digite um caminho
open-path-placeholder = uma pasta de arquivos .mid, .kar, .mp3+.cdg ou de vídeo
open-create-here = Criar aqui
open-recent-counts = { $songs ->
    [0] { $songs } músicas
    [one] { $songs } música
   *[other] { $songs } músicas
  } · { $files ->
    [0] { $files } arquivos
    [one] { $files } arquivo
   *[other] { $files } arquivos
  }

## Entrar numa máquina ---------------------------------------------------------------

access-signed-in = Conectado a esta máquina.
access-forget-it = Esquecer
access-debugging-off = Desligar a depuração
access-debugging-on = Ligar a depuração
access-what-signing-in-buys = Enviar e instalar um pacote exigem acesso de administrador, que esta conexão dá. A depuração é um ajuste separado, na própria máquina, e o botão Tocar precisa dela para ouvir uma música numa máquina que não seja este computador. A mudança vale quando aquela máquina reiniciar, então o botão acima mostra com qual ajuste ela está rodando agora.
access-password-saved = Este computador tem a senha desta máquina.
access-retype = Digitar outra senha
access-password = Senha
access-password-placeholder = senha de administrador
access-remember-it = guardar neste computador
access-sign-in = Conectar
access-which-password-lead = A senha de administrador da máquina, a mesma que a página
access-which-password-tail = dela pede. Se nenhuma senha foi definida, use o código de seis dígitos que aparece na tela da máquina. A senha é trocada por um token guardado na memória até este programa fechar.
access-tick-to-save = Marque a caixa para guardar a senha na pasta de configuração deste computador, sob o identificador desta máquina. Ela nunca é gravada na pasta do acervo.
access-owner-only = O arquivo só pode ser lido por você.
access-no-protection = O arquivo não tem proteção além da sua pasta de perfil.
access-nothing-to-save-for = Nenhuma máquina respondeu neste endereço ainda, então não há senha para guardar. Aperte “Salvar e verificar” primeiro.

## Arquivos que parecem uma mesma gravação ----------------------------------------------

duplicates-heading = Arquivos que parecem uma mesma gravação
duplicates-none-lead = Nada agrupado ainda. A comparação põe a forma de cada música contra a de todas as outras e precisa que uma leitura já as tenha lido; num acervo de algumas centenas de milhares de arquivos leva alguns segundos.
duplicates-none-tail = termina com ela, então apertar aqui é para o tempo entre uma leitura e outra.
duplicates-found = { $groups ->
    [0] { $groups } grupos de arquivos parecem uma mesma gravação, e a lista mostra a melhor cópia de cada um — escondendo { $hidden } músicas que seriam organizadas duas vezes.
    [one] { $groups } grupo de arquivos parece uma mesma gravação, e a lista mostra a melhor cópia de cada um — escondendo { $hidden } músicas que seriam organizadas duas vezes.
   *[other] { $groups } grupos de arquivos parecem uma mesma gravação, e a lista mostra a melhor cópia de cada um — escondendo { $hidden } músicas que seriam organizadas duas vezes.
  }
duplicates-nothing-to-judge = Nada foi juntado e nada precisa ser julgado. Qual arquivo de um grupo é a mesma gravação se descobre ouvindo, então o grupo aparece na página da própria música com um botão de tocar ao lado de cada versão, e “todas as versões” na barra de filtros desliga o agrupamento.
duplicates-look-again = Procurar de novo
duplicates-look-again-note = Lê o acervo inteiro, e a resposta substitui a anterior — um par que não corresponde mais é descartado. Um par descartado como diferente continua descartado.
duplicates-identical-heading = Cópias idênticas byte a byte
duplicates-identical-note = Nada a decidir aqui, e nada a rodar. A identidade de uma música é o hash dos bytes dela, então arquivos idênticos já são uma música com vários caminhos, e a coluna Cópias diz quantos.
duplicates-more-than-one-copy = Músicas com mais de uma cópia, as de mais primeiro

## Pacotes --------------------------------------------------------------------------

packages-new = Novo pacote
packages-name = Nome
packages-name-placeholder = Rock Clássico Vol 1
packages-volumes = { $count ->
    [0] { $count } volumes
    [one] { $count } volume
   *[other] { $count } volumes
  }
packages-version = Versão
packages-version-title = Três números separados por pontos, como 1.0.0
packages-publisher = Quem publica
packages-first-number = Primeiro número
packages-numbering = O identificador do pacote é gerado e nunca muda, então montar de novo com mais músicas produz o mesmo pacote. O número de uma música dentro de um pacote vai de 1 a { $highest }, e a máquina acrescenta os milhares que dizem de qual pacote ela é, então nada digitado aqui pode bater com um pacote já instalado.
packages-create = Criar
packages-open = Abrir pacote
packages-path-to = Caminho de um
packages-import-note = As músicas são ligadas a este acervo pelo hash do conteúdo, e as correções salvas no pacote também entram. Entradas cujos arquivos não estão dentro da pasta do acervo são listadas, e não puladas em silêncio.
packages-import = Importar
packages-from = A partir de
packages-last-built = Montado em
packages-never = nunca
packages-delete-confirm = Excluir o pacote { $id }? As músicas e qualquer .kmpkg já gravado não são alterados.

## Um pacote ------------------------------------------------------------------------

package-id-title = O identificador gerado deste pacote. Ele nunca muda.
package-id = identificador
package-volume-format = nome do volume
package-volume-format-title = Como o número de um volume é escrito depois do nome do pacote no arquivo e no manifesto. É escrito quando o pacote tem dois volumes, ou desde o primeiro quando a caixa abaixo está marcada.
package-volume-format-note = {"{"}n{"}"} é o número: vol{"{"}n{"}"} dá vol1, vol2.
package-number-one-volume = numerar o volume mesmo quando há só um
package-number-one-volume-title = Para um pacote que vai passar de 999 músicas. O primeiro arquivo se chama vol1 desde a primeira geração, então o nome não muda quando começa um segundo volume.
package-find-songs = Procurar músicas para acrescentar
package-folder = pasta
package-file = arquivo do pacote
package-build = Montar o arquivo do pacote
package-build-all = Montar todos os volumes
package-build-all-note = Cada volume é gravado na pasta com o próprio nome padrão.
package-write-listing = gravar também uma lista das músicas ao lado
package-spec-file = arquivo da especificação
package-write-spec = Gravar especificação
package-spec-for = Especificação para
package-name = nome
package-version = versão
package-volume = volume
package-publisher = quem publica
package-first-number = primeiro número
package-default-language-title = preenche qualquer música deste pacote que não tenha língua própria. Muda o pacote, nunca as músicas.
package-unclassified-are = músicas sem língua são
package-refuse-to-build = recusar a montagem
package-in-this-corpus = neste acervo
package-every-language = todas as línguas
package-empty = Vazio.
package-member-count = { $count } de { $highest } músicas. Um pacote que vem de favoritos abre outro volume quando as listas passam disso.
package-volume-tab = { $count ->
    [0] Volume { $number } · { $count } músicas
    [one] Volume { $number } · { $count } música
   *[other] Volume { $number } · { $count } músicas
  }
package-number = Nº
package-source-missing = origem sumiu
package-numbering-note = Mudar um número salva quando o campo perde o foco. Um número já usado por outra música deste pacote é recusado; os números não são trocados de lugar automaticamente.

## Um pacote que vem das favoritas ---------------------------------------------

package-sourced-from = Vem das favoritas
package-no-sources-yet = Nenhuma lista ainda, então este é um pacote comum.
package-no-favorites-lead = Nenhuma favorita ainda. Crie uma na página de
package-every-list-is-a-source = Todas as listas que este pacote pode ler já são fonte dele.
package-also-source-from = Vir também de
package-add-source = Adicionar
package-sources-note = Um pacote que vem de uma lista tem o que a lista tem, e não aparece onde se põem músicas num pacote uma a uma. Uma lista de trabalho não é oferecida a nenhum pacote: músicas que alguém separou para decidir depois não são um volume para montar.
sync-button = { $count ->
    [0] Sincronizar com { $count } listas…
    [one] Sincronizar com { $count } lista…
   *[other] Sincronizar com { $count } listas…
  }
sync-note = Põe o que as listas têm e tira o que elas não têm mais. Toda música que fica mantém o número dela.
sync-no-sources-note = Adicione uma lista acima para este pacote ter o que a lista tem.
sync-from = com
sync-adding = { $count ->
    [0] { $count } entram
    [one] { $count } entra
   *[other] { $count } entram
  }
sync-removing = { $count ->
    [0] { $count } saem
    [one] { $count } sai
   *[other] { $count } saem
  }
sync-keeping = { $count ->
    [0] { $count } ficam onde estão
    [one] { $count } fica onde está
   *[other] { $count } ficam onde estão
  }
sync-starts-volumes = { $count ->
    [0] todos os volumes estão cheios, então isto abre { $count } novos
    [one] todos os volumes estão cheios, então isto abre um novo
   *[other] todos os volumes estão cheios, então isto abre { $count } novos
  }
from-filter-keep-sourced = e ele continua vindo de

package-raise-first-build = subir a versão em toda montagem depois desta
package-raise-to = subir a versão para { $version } nesta montagem
package-raise-not-three-numbers = subir a versão — “{ $version }” não são três números, então não dá

## Ajustes ----------------------------------------------------------------------------

settings-machine = Máquina
settings-base-url = endereço
settings-save-and-check = Salvar e verificar
settings-discover = Procurar
settings-discover-note = Lista as máquinas encontradas nesta rede. Nada muda até você apertar “Usar”.
settings-machine-used-for = Usada para ouvir uma música de teste e para instalar um pacote pronto.
settings-here-lead = Esta máquina está neste computador, então a escuta de teste passa o caminho do arquivo e não copia nada. Ela só toca arquivos dentro das pastas listadas no ajuste
settings-here-tail = dela. Essa lista começa vazia, então a primeira escuta de teste numa instalação nova é recusada.
settings-elsewhere-lead = Esta máquina está em outro ponto da rede, então a escuta de teste envia a música, o que pode demorar no caso de um vídeo. Ela só aceita envios quando o ajuste
settings-elsewhere-tail = dela está ligado. Esse ajuste começa desligado, então a primeira escuta de teste numa máquina nova é recusada.
settings-which-setting-lead = A mensagem de erro diz qual ajuste mudar. Rode
settings-on-that-machine = naquela máquina
settings-which-setting-tail = para achar o arquivo de ajustes.

settings-backup = Cópia de segurança
settings-backup-to = gravar a cópia em
settings-write-backup = Gravar a cópia
settings-hand-set = { $count ->
    [0] { $count } músicas têm algo que você digitou.
    [one] { $count } música tem algo que você digitou.
   *[other] { $count } músicas têm algo que você digitou.
  }
settings-restore-from = restaurar de
settings-overwrite = substituir
settings-restore = Restaurar
settings-restore-confirm = Restaurar a partir deste arquivo?

settings-suggested-tags = Etiquetas sugeridas
settings-comma-separated = separadas por vírgula
settings-kept-in = Guardadas em
settings-kept-in-tail = , que você também pode editar à mão.

settings-this-folder = Esta pasta
settings-root = Raiz
settings-files = Arquivos
settings-did-not-parse = { $count ->
    [0] { $count } dos quais não foram lidos
    [one] { $count } dos quais não foi lido
   *[other] { $count } dos quais não foram lidos
  }

## A barra de filtros da lista ----------------------------------------------------------

songs-find-placeholder = título ou artista
songs-find-title = parte de um título ou de um artista; aspas pedem as palavras nessa ordem
songs-only-this-artist = só este artista
songs-artist-title = todas as músicas exatamente deste intérprete
songs-sort = ordenar por
songs-sort-title = título
songs-sort-artist = artista
songs-sort-suitability = adequação
songs-sort-length = duração
songs-sort-updated = mexidas há pouco
songs-sort-added = adicionadas há pouco
songs-your-score = a sua nota
songs-your-score-title = a nota que você deu, na coluna Nota
songs-copies = cópias
songs-copies-title = quantos arquivos idênticos byte a byte esta música tem no disco
songs-language = língua
songs-show-filenames = mostrar os nomes dos arquivos
songs-show-filenames-title = mostrar o nome do arquivo de cada música ao lado do título
songs-band-quality = Qualidade
songs-suitability-title = a qualidade do arquivo de origem
songs-any = qualquer
songs-unset = em branco
songs-set = preenchida
songs-melody-channel = canal de melodia
songs-melody-found = encontrado
songs-melody-not-found = não encontrado
songs-band-song = música
songs-media-type = tipo de mídia
songs-media-type-title = MIDI, vídeo, MP3+G, ou todos
songs-any-if-set = qualquer uma, se houver
songs-tags = etiquetas
songs-add-tag-title = ampliar para as músicas com esta etiqueta além das já escolhidas
songs-add-one = acrescentar uma
songs-suggested = sugerida
songs-lyrics = letra
songs-per-syllable = por sílaba
songs-per-line = por linha
songs-lyrics-none = nenhuma
songs-encoding = codificação
songs-encoding-title = "adivinhada" é o palpite CP1252, cerca de um terço de um acervo de verdade, e é o texto que vale a pena olhar
songs-encoding-guessed = adivinhada
songs-encoding-detected = detectada
songs-encoding-pinned = fixada
songs-band-state = situação
songs-favorite = lista
songs-filed = guardada
songs-filed-title = se a música está em alguma lista, em uma que seja arquivo, em nenhuma, em nenhuma definitiva, ou tanto faz
songs-in-any-favorite = em alguma lista
songs-in-filed-favorite = em lista definitiva, não de trabalho
songs-in-no-favorite = em nenhuma lista
songs-in-no-filed-favorite = em nenhuma lista definitiva, fora as de trabalho
songs-more-than-ten = mais de 10
songs-added = adicionada
songs-added-title = quando uma varredura encontrou esta música pela primeira vez
songs-added-day = no último dia
songs-added-week = nos últimos 7 dias
songs-added-month = nos últimos 30 dias
songs-added-over-month = há mais de 30 dias
songs-every-version = todas as versões
songs-every-version-title = mostrar todo arquivo de uma gravação, e não só a melhor cópia de cada
songs-not-packaged = fora de pacotes

kind-midi = MIDI
kind-video = vídeo
kind-cdg = MP3+G
kind-ultrastar = UltraStar

## As abas de curadoria -----------------------------------------------------------------

songs-tab-language = Língua
songs-tab-tags = Etiquetas
songs-tab-titles = Títulos
songs-tab-analysis = Análise
songs-scope-ticked = as músicas marcadas
songs-scope-matching = todas as músicas da lista
songs-to = para
songs-set-language-of = definir a língua de
songs-only-if-not-set = só se ainda não tiver
songs-tag-add = pôr
songs-tag-remove = tirar
songs-the-tag = a etiqueta
songs-tag-title = só letras ASCII, números e -; os acentos caem, então "Forró" vira "forro"
songs-apply = Aplicar
songs-make-package-called = fazer um pacote com todas as músicas da lista, chamado
songs-make = Fazer
songs-add = pôr
songs-no-packages-lead = Nenhum pacote ainda - faça um na página
songs-no-favorites-lead = Nenhuma lista ainda - faça uma na página
songs-page-tail = .
songs-put-into = pôr em
songs-take-out-of = tirar de
songs-title-from-filename = Título a partir do nome do arquivo
songs-title-from-filename-note = Troca o título das músicas marcadas pelo nome do arquivo delas, e limpa o artista que o arquivo declarava.
songs-fix-capitals = Arrumar as maiúsculas
songs-fix-capitals-note = Põe maiúscula em cada palavra do título e do artista das músicas marcadas, deixando pequenas as palavras curtas como "the" e "de". É uma primeira passada: um nome digitado por você que já mistura maiúsculas fica como está.
songs-split-artist = Artista a partir do título
songs-split-artist-note = Divide o título das músicas marcadas no primeiro "-" e põe no artista o que vem antes dele. Só uma música que está sem artista.
songs-recalculate-of = recalcular a adequação de
songs-recalculate = Recalcular
songs-recalculate-note = Lê cada arquivo de novo. O que você digitou fica como está.
songs-hint = Sugerir qual tocar primeiro
songs-hint-clear = Limpar
songs-hint-note = Numera as músicas MIDI marcadas 1, 2, 3... A lista não se mexe e nada é salvo.

## A página de uma música ------------------------------------------------------------

song-back = voltar para a lista
song-no-title-title = o arquivo não traz título - mostrando o nome dele
song-test-play = Ouvir
song-open-in-os = Abrir no sistema
song-similar = Nomes parecidos
song-download = Baixar
song-youtube = YouTube
song-favorites-title = as listas em que esta música está

song-merged-lead = Esta música está marcada como a mesma gravação de
song-another-song = outra música
song-merged-tail = , então fica escondida da lista.
song-unmerge = Desfazer
song-duplicate-lead = Outro arquivo parece a mesma gravação e é melhor, então a lista mostra
song-that-one = aquele
song-duplicate-tail = no lugar deste. Ninguém decidiu isso; foi deduzido dos dois arquivos.
song-show-anyway = Mostrar mesmo assim
song-show-anyway-title = voltar a listar este arquivo sozinho, até a próxima comparação

song-tab-details = Detalhes
song-tab-files = Arquivos
song-tab-filing = Organização
song-tab-lyrics = Letra
song-tab-advanced = Avançado

package-tab-songs = Músicas
package-tab-sources = Fontes
package-tab-build = Montagem

song-file-says-nothing = (o arquivo não diz)
song-language-declared = Em branco, então esta música conta como { $name } - o cabeçalho do arquivo diz { $code }.
song-language-declared-default = Em branco, então esta música conta como { $name } - o cabeçalho do arquivo diz { $code }, que é o que a maioria dos arquivos de karaokê diz, seja qual for a língua.
song-language-from-encoding = Em branco, então esta música conta como { $name } - deduzido da codificação em que a letra está escrita.
song-language-unknown-code = O cabeçalho do arquivo diz { $code }, que não é um código de língua que esta versão conheça.

song-transpose = Transpor
song-semitones = semitons, por padrão
song-notes = Anotações
song-correction-note = vale toda vez que esta música toca, aqui e na máquina

song-file-analysis = Análise do arquivo
song-file-info = Dados do arquivo
song-suitability-parts = letra { $lyrics }/3 - sincronia { $sync }/3 - canais { $channels }/2 - arranjo { $arrangement }/2
song-native-karaoke = Arquivo de karaokê de verdade.
song-your-rating = A sua nota
song-melody-channel = canal { $channel }
song-melody-channel-confidence = canal { $channel } (confiança { $confidence })
song-melody-not-found = não encontrado
song-melody-no-candidates = não encontrado - nenhum instrumento toca nota alguma
song-melody-nothing-monophonic = não encontrado - nenhum canal toca uma nota por vez
song-melody-outside-vocal-range = não encontrado - todo canal que toca uma nota por vez está fora da extensão da voz
song-melody-silent-under-the-words = não encontrado - nenhum canal toca enquanto a letra é cantada
song-melody-no-supporting-evidence = não encontrado - nada liga um canal que toca uma nota por vez à letra
song-melody-ambiguous = não encontrado - dois ou mais canais servem igualmente bem
song-format = Formato
column-length-full = Duração
song-content-label = Conteúdo
song-content = { $notes } notas em { $channels } canais - { $lines } linhas, { $syllables } sílabas
song-encoding = Codificação
song-encoding-guess = É um palpite. Leia a letra na aba Letra e fixe a codificação certa se o texto estiver errado.
song-picture = Imagem
song-codecs = Codecs
song-audio = áudio
song-graphics-file = Arquivo de gráficos
song-graphics = Gráficos
song-cdg-length = a letra corre por { $words }
song-cdg-audio = { $channels ->
    [0] { $channels } canais
    [one] { $channels } canal
   *[other] { $channels } canais
  }
song-cdg-graphics = { $tiles } blocos - { $packets } pacotes
song-cdg-unknown = { $count ->
    [0] { $count } pacotes usam uma instrução CD+G que esta versão não implementa.
    [one] { $count } pacote usa uma instrução CD+G que esta versão não implementa.
   *[other] { $count } pacotes usam uma instrução CD+G que esta versão não implementa.
  }
song-added = Adicionada em
song-hash = Hash
song-warnings = Avisos

song-files-heading = { $count ->
    [0] Arquivos no disco - { $count } cópias idênticas
    [one] Arquivos no disco
   *[other] Arquivos no disco - { $count } cópias idênticas
  }
song-no-files = Nenhum arquivo desta música está mais dentro da pasta do acervo. Ela fica porque um pacote ainda a nomeia; leia de novo depois de restaurar a pasta, ou tire-a do pacote.
song-bytes = bytes
song-folder-title = todas as músicas desta pasta, subpastas incluídas
song-songs-in-folder = músicas nesta pasta
song-versions-heading = { $count ->
    [0] Outras versões - { $count }
    [zero] Outras versões
   *[other] Outras versões - { $count }
  }
song-no-other-versions = Nenhum outro arquivo aqui parece esta gravação.
song-other-versions-note = Bytes diferentes, a mesma forma e o mesmo nome. A lista mostra um destes e esconde os outros, para nada ser organizado duas vezes. Qual deles é de fato a mesma gravação se descobre ouvindo, e é para isso que servem os botões de tocar.
song-test-play-version-title = ouvir, para saber se é a mesma gravação
song-not-the-same = Não é a mesma
song-not-the-same-title = parar de agrupar estas duas, para sempre

song-tag-title = o que você digitar vira um identificador simples, e só letras ASCII, números e - sobrevivem
song-no-favorites-defined = Nenhuma lista criada.
song-save-favorites = Salvar as listas
song-no-packages = Em nenhum pacote.
song-as-number = com o número
song-add-to-package = Pôr num pacote
song-replace-number = Pôr no lugar do número
song-replace-in = em
song-replace = Trocar
song-replace-title = Dá a esta música o número que outra música tem num pacote. A outra música sai do pacote, e o número continua o mesmo.

song-decode-as = ler como
song-show = Mostrar
song-loading = carregando
song-text-events = Eventos de texto deste arquivo
song-show-raw = Mostrar o texto cru

song-channels = Canais
song-channel = Canal
song-instrument-in-file = Instrumento no arquivo
song-notes-column = Notas
song-track = Trilha
song-ignore-bank = Ignorar a seleção de banco
song-silence = Silenciar
song-recentre-bend = Corrigir bend preso
song-recentre-bend-title = Esta parte entorta uma nota e não a desentorta por completo, então as notas seguintes tocam desafinadas. Marque para desfazer o bend antes dessas notas.
song-play-as = Tocar como
song-drums = bateria
song-no-melody = sem melodia
song-melody-is-this = a melodia está neste canal
song-a-kit = um kit, não um instrumento
song-save-corrections = Salvar as correções

## O que uma ação responde --------------------------------------------------------------

said-added-to-favorite = Guardada nessa lista.
said-build-it-first = Monte o pacote primeiro — não há arquivo para instalar.
said-choose-a-favorite = Escolha uma lista primeiro.
said-corrections-saved = Correções salvas.
said-favorite-created = { $name } criada. Marque músicas nela pela página Músicas.
said-favorite-renamed = Renomeada para { $name }.
said-favorite-deleted = Excluída. As músicas que ela tinha ficam como estão.
said-favorite-gone = Essa lista sumiu. Recarregue a página.
said-filter-already-gone = Esse já não existe.
said-filter-renamed = Renomeado para { $name }.
said-filter-saved = Salvo como { $name }.
said-filter-saved-not-drawn = { $name } foi salvo. A faixa não pôde ser redesenhada: { $error }
said-filter-updated = { $name } foi atualizado.
said-forgotten = Esquecido.
said-in-no-favorites = Em nenhuma lista.
said-name-first = Dê um nome primeiro.
said-name-the-backup = Diga de qual arquivo de cópia restaurar.
said-name-the-package = Dê um nome ao pacote primeiro.
said-no-folder-given = Nenhuma pasta foi indicada.
said-no-folder-open = Nenhuma pasta está aberta.
said-no-numbers-to-clear = Não há números para limpar.
said-not-a-number = Isso não é um número.
said-now-number = Agora é o número { $number }.
said-choose-a-package = Escolha um pacote primeiro.
confirm-replace = O número { $number } em { $package } é { $old }. Pôr { $new } no lugar?
confirm-replace-favorites = { $count ->
    [0] Este pacote segue as listas { $favorites }, então { $new } também entra no lugar nelas.
    [one] Este pacote segue a lista { $favorites }, então { $new } também entra no lugar nela.
   *[other] Este pacote segue as listas { $favorites }, então { $new } também entra no lugar nelas.
  }
said-replaced = { $new } agora é o número { $number } em { $package }, no lugar de { $old }.
said-replaced-in-favorites = Também entrou no lugar em { $favorites }.
said-replace-empty = Nenhuma música tem o número { $number } em { $package }.
said-replace-same-song = Esta música já tem o número { $number } ali.
said-replace-already-in = Esta música já está nesse pacote, com o número { $number } em { $package }. Tire-a de lá primeiro, ou troque outra música por ela.
said-replace-merged = Esta música foi juntada a outra. Troque pela outra.
said-nothing-change = Nada a mudar.
said-nothing-is-ticked = Nada está marcado.
said-nothing-matches-filter = Nada corresponde a esse filtro.
said-nothing-to-drop = Nada a tirar: cada música dela é uma gravação diferente.
said-nothing-was-ticked = Nada foi marcado.
said-opened-in-browser = Aberto no seu navegador.
said-package-created = { $name } criado. Abra para escolher o que entra e para montar.
said-package-deleted = Excluído. Qualquer .kmpkg já gravado fica como está.
said-rated = Nota { $value }/10.
said-rating-cleared = Nota apagada.
said-removed-from-favorite = Tirada dessa lista.
said-removed = Tirado deste pacote. A música em si fica como está.
said-renumbered = { $count ->
    [0] { $count } músicas renumeradas a partir do primeiro número deste pacote, mantendo a ordem.
    [one] { $count } música renumerada a partir do primeiro número deste pacote, mantendo a ordem.
   *[other] { $count } músicas renumeradas a partir do primeiro número deste pacote, mantendo a ordem.
  }
said-saved = Salvo.
said-volume-format-refused = O nome do volume precisa ter {"{"}n{"}"}, onde vai o número, ou todos os volumes seriam gravados num arquivo só. “{ $format }” não foi salvo.
said-saved-no-tags = Salvo. Nenhuma etiqueta é sugerida agora.
said-suggestion-pass-unfinished = a comparação não terminou
said-type-a-tag = Digite uma etiqueta primeiro.
said-unmerged = Desfeito. Ela volta a aparecer na lista.

## ...e o que uma ação responde quando conta alguma coisa ------------------------------

said-language-set = { $count ->
    [0] A língua de { $count } músicas foi definida.
    [one] A língua de { $count } música foi definida.
   *[other] A língua de { $count } músicas foi definida.
  }
said-tag-set = { $count ->
    [0] A etiqueta foi posta em { $count } músicas.
    [one] A etiqueta foi posta em { $count } música.
   *[other] A etiqueta foi posta em { $count } músicas.
  }
said-tag-removed = { $count ->
    [0] A etiqueta foi tirada de { $count } músicas.
    [one] A etiqueta foi tirada de { $count } música.
   *[other] A etiqueta foi tirada de { $count } músicas.
  }
said-filed = { $count ->
    [0] { $count } músicas guardadas.
    [one] { $count } música guardada.
   *[other] { $count } músicas guardadas.
  }
said-took-out = { $count ->
    [0] { $count } músicas tiradas.
    [one] { $count } música tirada.
   *[other] { $count } músicas tiradas.
  }
said-re-reading = { $count ->
    [0] Relendo { $count } arquivos. Acompanhe na página Leitura.
    [one] Relendo { $count } arquivo. Acompanhe na página Leitura.
   *[other] Relendo { $count } arquivos. Acompanhe na página Leitura.
  }
said-cleared-numbers = { $count ->
    [0] { $count } números limpos.
    [one] { $count } número limpo.
   *[other] { $count } números limpos.
  }
said-not-a-tag = { $typed } não é uma etiqueta — uma etiqueta é feita de letras ASCII, números e hífen. Os acentos caem, então um o acentuado vira um o simples; o resto não passa.
said-encoding-pinned = { $encoding } fixada. Todo pacote montado com esta música vai lê-la assim.
said-now-a-working-list = { $name } agora é uma lista de trabalho. As músicas dela não contam mais como arquivadas.
said-now-a-filing = { $name } voltou a ser um arquivamento.
said-dropped-second-copies = { $count ->
    [0] { $count } entradas tiradas. A melhor cópia que a lista guardava de cada música ficou.
    [one] { $count } entrada tirada. A melhor cópia que a lista guardava de cada música ficou.
   *[other] { $count } entradas tiradas. A melhor cópia que a lista guardava de cada música ficou.
  }
said-grouping-done = { $groups } grupos parecem uma mesma gravação, escondendo { $hidden } músicas. { $pairs } pares.
said-version-refused = Uma versão são três números separados por pontos — 1.0.0, por exemplo. “{ $version }” não é, e não foi salva.
said-start-number-refused = Um pacote numera as músicas de 1 a { $highest }; a máquina acrescenta o bloco, então um primeiro número acima disso seria discado como de outro pacote. { $number } não foi salvo.
said-package-full = { $name } não tem mais números. Um pacote guarda { $highest } músicas e este está cheio.
said-name-has-no-file-name = { $name } não deixa nada que sirva de nome de arquivo. Tente um nome com letras ou números.
said-spec-written = { $count ->
    [0] { $file } gravado, descrevendo { $count } músicas. Edite em qualquer editor de texto e monte com o km-pack, ou continue montando por esta página, que lê a mesma descrição sem o arquivo.
    [one] { $file } gravado, descrevendo { $count } música. Edite em qualquer editor de texto e monte com o km-pack, ou continue montando por esta página, que lê a mesma descrição sem o arquivo.
   *[other] { $file } gravado, descrevendo { $count } músicas. Edite em qualquer editor de texto e monte com o km-pack, ou continue montando por esta página, que lê a mesma descrição sem o arquivo.
  }
said-build-gone = { $file } não está mais lá. Monte de novo.
said-imported-all = { $count ->
    [0] { $package } importado com { $count } músicas, todas ligadas a arquivos desta pasta.
    [one] { $package } importado com { $count } música, ligada a um arquivo desta pasta.
   *[other] { $package } importado com { $count } músicas, todas ligadas a arquivos desta pasta.
  }
said-imported = { $count ->
    [0] { $package } importado com { $count } músicas.
    [one] { $package } importado com { $count } música.
   *[other] { $package } importado com { $count } músicas.
  }
said-machine-reached = Salvo. Falei com “{ $name }” em { $url }.
said-machine-saved-no-answer = { $url } salvo, mas nenhuma máquina respondeu lá ainda: { $why }
said-backup-written = { $file } gravado: { $songs ->
    [0] { $songs } músicas
    [one] { $songs } música
   *[other] { $songs } músicas
  } com algo que você digitou, e { $favorites ->
    [0] { $favorites } listas
    [one] { $favorites } lista
   *[other] { $favorites } listas
  }. Guarde fora desta pasta — é a metade deste acervo que uma nova leitura não refaz.
said-restored = { $songs ->
    [0] { $songs } músicas restauradas
    [one] { $songs } música restaurada
   *[other] { $songs } músicas restauradas
  }, { $filed } guardadas em { $favorites ->
    [0] { $favorites } listas novas
    [one] { $favorites } lista nova
   *[other] { $favorites } listas novas
  }, e { $merges ->
    [0] { $merges } junções registradas
    [one] { $merges } junção registrada
   *[other] { $merges } junções registradas
  }.
said-restored-later-format = Este arquivo foi gravado por uma versão mais nova (formato { $format }); tudo o que esta entende foi lido mesmo assim.

## O que uma etiqueta diz sobre um filtro -------------------------------------------------

chip-by = de { $artist }
chip-starts-with = começa com { $letter }
chip-starts-with-a-number = começa com número
chip-starts-with-a-symbol = começa com símbolo
initial-any = qualquer
initial-symbol = símbolo
chip-suitability-high = adequação 8–10
chip-suitability-middle = adequação 5–7
chip-suitability-low = adequação abaixo de 5
chip-score-set = { $name } preenchida
chip-score-unset = { $name } em branco
chip-score-at-least = { $name } ≥ { $score }
chip-melody-found = melodia encontrada
chip-melody-abstained = melodia sem resposta
chip-midi-only = só MIDI
chip-video-only = só vídeo
chip-cdg-only = só MP3+G
chip-language-unset = sem língua
chip-language-any = com alguma língua
chip-tag = etiqueta: { $tag }
chip-lyrics-per-syllable = letra por sílaba
chip-lyrics-per-line = letra por linha
chip-no-lyrics = sem letra
chip-in-favorite = em { $name }
chip-one-copy = uma cópia
chip-two-to-ten-copies = 2–10 cópias
chip-over-ten-copies = mais de 10 cópias
chip-added-day = adicionada no último dia
chip-added-week = adicionada nos últimos 7 dias
chip-added-month = adicionada nos últimos 30 dias
chip-added-over-month = adicionada há mais de 30 dias
chip-more-than-one-copy = mais de uma cópia

## ...e o resto do que uma ação responde --------------------------------------------------

said-no-address-yet = O programa ainda não sabe o próprio endereço.
said-nothing-ticked-on-disk = Nada está marcado, ou nenhuma música marcada ainda tem arquivo no disco.
said-nothing-matching-on-disk = Nada corresponde a esse filtro, ou nada nele ainda tem arquivo no disco.
said-titles-taken = { $count ->
    [0] O título de { $count } músicas veio do nome dos arquivos, e o artista foi limpo.
    [one] O título de { $count } música veio do nome do arquivo, e o artista foi limpo.
   *[other] O título de { $count } músicas veio do nome dos arquivos, e o artista foi limpo.
  }
said-titles-taken-short = { $count ->
    [0] O título de { $count } músicas veio do nome dos arquivos, e o artista foi limpo; { $short } não tinham nome de arquivo para usar.
    [one] O título de { $count } música veio do nome do arquivo, e o artista foi limpo; { $short } não tinha nome de arquivo para usar.
   *[other] O título de { $count } músicas veio do nome dos arquivos, e o artista foi limpo; { $short } não tinham nome de arquivo para usar.
  }
said-capitals-fixed = { $count ->
    [0] As maiúsculas de { $count } músicas foram arrumadas.
    [one] As maiúsculas de { $count } música foram arrumadas.
   *[other] As maiúsculas de { $count } músicas foram arrumadas.
  }
said-capitals-fixed-short = { $count ->
    [0] As maiúsculas de { $count } músicas foram arrumadas; { $short } já tinham maiúsculas próprias.
    [one] As maiúsculas de { $count } música foram arrumadas; { $short } já tinha maiúsculas próprias.
   *[other] As maiúsculas de { $count } músicas foram arrumadas; { $short } já tinham maiúsculas próprias.
  }
said-artist-split = { $count ->
    [0] O artista saiu do título de { $count } músicas.
    [one] O artista saiu do título de { $count } música.
   *[other] O artista saiu do título de { $count } músicas.
  }
said-artist-split-short = { $count ->
    [0] O artista saiu do título de { $count } músicas; { $short } já tinham artista ou não tinham "-" no título.
    [one] O artista saiu do título de { $count } música; { $short } já tinha artista ou não tinha "-" no título.
   *[other] O artista saiu do título de { $count } músicas; { $short } já tinham artista ou não tinham "-" no título.
  }
said-nothing-ticked-is-midi = Nada do que está marcado é um arquivo MIDI, e um vídeo ou um MP3+G é 10 pelo que é, e não por medição.
said-numbered = { $count ->
    [0] { $count } músicas numeradas, as melhores primeiro.
    [one] { $count } música numerada, a melhor primeiro.
   *[other] { $count } músicas numeradas, as melhores primeiro.
  }
said-numbered-skipped = { $count ->
    [0] { $count } músicas numeradas, as melhores primeiro. { $skipped } não são arquivos MIDI e não têm o que comparar.
    [one] { $count } música numerada, a melhor primeiro. { $skipped } não são arquivos MIDI e não têm o que comparar.
   *[other] { $count } músicas numeradas, as melhores primeiro. { $skipped } não são arquivos MIDI e não têm o que comparar.
  }
said-handed-to-opener = { $file } foi entregue ao sistema para abrir. Isso acontece na máquina que está rodando o package builder, então se o seu navegador está em outro lugar, use Baixar.
said-could-not-open = não foi possível abrir: { $why }
said-worker-died = a linha de trabalho morreu: { $why }
said-shown-again = Volta a aparecer na lista. A próxima comparação a esconde de novo; descarte o par em vez disso.
said-pair-dismissed = Anotado. As duas não serão mais agrupadas, e cada uma aparece por si.

## O que uma montagem, uma importação ou uma restauração relata -------------------------

said-package-needs-a-name = Um pacote precisa de um nome — é por ele que você vai reconhecê-lo, e o identificador é gerado.
said-default-language-set = Salvo. As músicas sem língua própria entram como { $code } — só no pacote; nada é gravado de volta nas músicas.
said-default-language-cleared = Salvo. Uma música sem língua agora impede a montagem e aparece listada para você classificar.
said-package-made = { $count ->
    [0] { $package } feito com { $count } músicas. Está na página Pacotes, onde é montado.
    [one] { $package } feito com { $count } música. Está na página Pacotes, onde é montado.
   *[other] { $package } feito com { $count } músicas. Está na página Pacotes, onde é montado.
  }
said-package-made-sourced = Ele continua vindo daquela lista, então não aparece onde se põem músicas num pacote uma a uma, e Sincronizar, na página dele, é o que mantém os dois juntos.
said-package-took-the-first = O filtro pegou mais músicas do que um pacote comporta, então as { $count } primeiras desta ordem entraram.
said-package-no-room = { $count ->
    [0] { $count } delas ficaram sem número.
    [one] { $count } delas ficou sem número.
   *[other] { $count } delas ficaram sem número.
  }
said-name-a-list = Escolha uma lista primeiro.
said-source-added = Agora vem de { $name }. Aperte Sincronizar para o pacote ter o que as listas dele têm.
said-source-removed = Não vem mais de { $name }. As músicas que ela pôs continuam lá até a próxima sincronização.
said-source-is-a-working-list = { $name } é uma lista de trabalho, e nenhum pacote recebe uma: músicas que alguém separou para decidir depois não são um volume para montar. Tire a marca de lista de trabalho dela na página Favoritas, ou escolha outra.
said-sources-cleared = Este pacote não vem de nenhuma lista agora, então voltou a ser um pacote comum e dá para pôr música nele uma a uma. As músicas que ele tem ficam onde estão.
said-no-sources-to-sync = Este pacote não vem de nenhuma lista, e sincronizar assim o esvaziaria. Adicione uma lista primeiro.
said-nothing-to-sync = { $count ->
    [0] Já tem o que as listas dele têm — { $count } músicas, nada a pôr e nada a tirar.
    [one] Já tem o que as listas dele têm — { $count } música, nada a pôr e nada a tirar.
   *[other] Já tem o que as listas dele têm — { $count } músicas, nada a pôr e nada a tirar.
  }
said-synced = Sincronizado: { $added } entraram, { $removed } saíram, { $kept } ficaram com os números delas.
said-sync-new-volumes = { $count ->
    [0] As listas passaram do tamanho do pacote, então ele tem { $count } volumes novos.
    [one] As listas passaram do tamanho do pacote, então ele tem um volume novo.
   *[other] As listas passaram do tamanho do pacote, então ele tem { $count } volumes novos.
  }
said-sync-clashed = { $count ->
    [0] { $count } delas são outro arquivo de uma música que o pacote já tinha — ficaram, porque duas gravações de uma música podem ser duas músicas. Limpar, na página Favoritas, é o que tira as cópias repetidas.
    [one] { $count } delas é outro arquivo de uma música que o pacote já tinha — ficou, porque duas gravações de uma música podem ser duas músicas. Limpar, na página Favoritas, é o que tira as cópias repetidas.
   *[other] { $count } delas são outro arquivo de uma música que o pacote já tinha — ficaram, porque duas gravações de uma música podem ser duas músicas. Limpar, na página Favoritas, é o que tira as cópias repetidas.
  }
said-package-is-sourced = { $count ->
    [0] Esse pacote tem o que { $favorites } têm, então uma música posta aqui sairia de novo na próxima sincronização. Ponha a música numa daquelas listas.
    [one] Esse pacote tem o que { $favorites } tem, então uma música posta aqui sairia de novo na próxima sincronização. Ponha a música naquela lista.
   *[other] Esse pacote tem o que { $favorites } têm, então uma música posta aqui sairia de novo na próxima sincronização. Ponha a música numa daquelas listas.
  }

said-build-stopped = Parou antes de gravar qualquer coisa. O pacote que seria substituído ficou como estava.
said-build-volume = Volume { $number }:
said-build-nothing-readable = Nada foi gravado: nenhuma música deste pacote pôde ser lida.
said-build-manifest-problems = Não foi gravado — o manifesto tem problemas:
said-build-unlanguaged = { $count ->
    [0] Não foi gravado — { $count } músicas estão sem língua, e um pacote não sai sem isso:
    [one] Não foi gravado — { $count } música está sem língua, e um pacote não sai sem isso:
   *[other] Não foi gravado — { $count } músicas estão sem língua, e um pacote não sai sem isso:
  }
said-build-unlanguaged-ways-out = Ou defina “músicas sem língua são” acima — o que as preenche só neste pacote e não grava nada de volta nas músicas — ou classifique-as de vez na página Músicas: filtre pelas que estão sem língua, acrescente a pasta em que estão, e use “definir a língua de todas as músicas da lista”.
said-build-written = { $count ->
    [0] { $file } gravado, versão { $version }, com { $count } músicas
    [one] { $file } gravado, versão { $version }, com { $count } música
   *[other] { $file } gravado, versão { $version }, com { $count } músicas
  }
said-build-listing-written = A lista das músicas está em { $file }.
said-build-re-encoded = { $count ->
    [0] { $count } vídeos recodificados
    [one] { $count } vídeo recodificado
   *[other] { $count } vídeos recodificados
  }
said-build-copied = { $count ->
    [0] { $count } copiados como estavam
    [one] { $count } copiado como estava
   *[other] { $count } copiados como estavam
  }
said-build-cdg-pairs = { $count ->
    [0] { $count } pares MP3+G
    [one] { $count } par MP3+G
   *[other] { $count } pares MP3+G
  }
said-build-ultrastar = { $count ->
    [0] { $count } músicas UltraStar
    [one] { $count } música UltraStar
   *[other] { $count } músicas UltraStar
  }
said-build-left-out = { $count ->
    [0] { $count } músicas de fora:
    [one] { $count } música de fora:
   *[other] { $count } músicas de fora:
  }
said-and-more = ... e mais { $count }
said-import-unmatched = { $count ->
    [0] { $count } não puderam ser ligadas a nada dentro desta pasta:
    [one] { $count } não pôde ser ligada a nada dentro desta pasta:
   *[other] { $count } não puderam ser ligadas a nada dentro desta pasta:
  }
said-import-unreadable-language = { $count ->
    [0] { $count } músicas indicam uma língua que esta versão não lê, então não foram importadas:
    [one] { $count } música indica uma língua que esta versão não lê, então não foi importada:
   *[other] { $count } músicas indicam uma língua que esta versão não lê, então não foram importadas:
  }
said-install-already-in-catalog = { $count ->
    [0] { $count } músicas já estão no catálogo:
    [one] { $count } música já está no catálogo:
   *[other] { $count } músicas já estão no catálogo:
  }
said-restore-unmatched = { $count ->
    [0] { $count } músicas do arquivo não estão nesta pasta — leia a pasta primeiro e restaure de novo:
    [one] { $count } música do arquivo não está nesta pasta — leia a pasta primeiro e restaure de novo:
   *[other] { $count } músicas do arquivo não estão nesta pasta — leia a pasta primeiro e restaure de novo:
  }
said-restore-rejected = { $count ->
    [0] { $count } valores que esta versão não aceita:
    [one] { $count } valor que esta versão não aceita:
   *[other] { $count } valores que esta versão não aceita:
  }
said-type-the-password = Digite a senha da máquina primeiro.
said-signed-in-with-saved = Conectado com a senha que este computador tinha guardada.
said-signed-in-remembered = Conectado. Este computador vai lembrar a senha desta máquina.
said-signed-in-not-written = Conectado. A senha não fica guardada.
said-signed-out = Desconectado, e nada fica guardado para esta máquina.

said-package-name-taken = Já existe um pacote chamado { $name }. Dê outro nome a este.
said-debugging-on = A depuração estará ligada quando esta máquina reiniciar. Até lá as duas rotas de toque não existem, então uma escuta de teste continua recusada.
said-debugging-off = A depuração estará desligada quando esta máquina reiniciar.

## Quando algo dá errado ---------------------------------------------------------------

db-error-sqlite = O banco de dados respondeu com um erro: { $why }
db-error-not-found = Não existe { $what } aqui.
db-error-no-folder = Nenhuma pasta está aberta.
db-error-busy = O acervo está sendo gravado. Tente novamente em um instante.
app-error-unreachable = A máquina de karaokê não responde em { $url } ({ $why }).
app-error-unexpected = A máquina de karaokê respondeu { $status }: { $body }
said-folder-not-listed = Não foi possível listar a pasta: { $why }
row-path-copies = { $path }
    { $count } cópias

## A janela que uma falha antes da página mostra -------------------------------------------

window-could-not-start = { $program } não conseguiu iniciar

said-machine-reached-at = Falei com “{ $name }” em { $url }.
said-machine-holds = { $count ->
    [0] Ela tem { $count } músicas instaladas.
    [one] Ela tem { $count } música instalada.
   *[other] Ela tem { $count } músicas instaladas.
  }
said-machine-stale = Nenhuma máquina responde neste endereço há seis horas ou mais. Se este acervo mudou de computador, aperte Procurar e escolha uma máquina desta rede.
