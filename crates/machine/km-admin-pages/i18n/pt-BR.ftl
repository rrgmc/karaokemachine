# O que as páginas de `/admin/` dizem.
#
# Tradução de `en.ftl`, que é onde toda mensagem é escrita primeiro.
#
# **Um leitor diferente do cantor.** Aqui é quem instalou a máquina, sentado na frente dela de
# propósito — então uma frase pode explicar uma consequência, o que um aviso num celular não pode.

## As páginas

admin-title = Configurando a máquina de karaokê
tab-machine = Esta máquina
tab-songs = Músicas
tab-sound = Som
tab-pictures = Imagens

## Os painéis de Esta máquina

pane-general = Geral
pane-access = Acesso
pane-singers = Cantores e fila
pane-playback = Reprodução
pane-developer = Desenvolvedor
pane-singers-none = Esta máquina não tem ajustes de cantores.

## Entrar

sign-in = Entrar
password-field = A senha
password-placeholder = pelo menos 4 caracteres

## Este programa entrando na máquina

login-yes = Este programa está conectado.
login-no = Este programa não está conectado, então nada aqui pode ser alterado ainda.
login-needed = Esta máquina pede a senha antes que algo possa mudar. Digite-a aqui.
login-forgotten = Este computador esqueceu essa senha.

## A máquina

machine-name = Nome da máquina
machine-where = Onde encontrá-la
machine-unreachable = Máquina não está acessível na rede.
machine-open-hint = { $addresses ->
    [one] Abra este endereço num celular para procurar e escolher músicas.
   *[other] Abra um destes num celular para procurar e escolher músicas.
 }
password-change = Trocar
password-change-warning = Esta máquina tem senha. Trocá-la desconecta todos os navegadores, inclusive este.

## Músicas

songs-add = Adicionar músicas
songs-empty = Nenhuma música ainda. Adicione um pacote acima.
column-package = Pacote
column-songs = Músicas
column-numbers = Números
column-bank = Faixa
column-move = Mover
package-bank = Banco do pacote { $package }
package-remove = Remover { $package }

## Som

bank-add = Adicionar um banco de sons
bank-add-hint = Um SoundFont decide como os instrumentos soam. Um arquivo .sf2, até 1 GB.
banks-here = Bancos nesta máquina
bank-use = Usar este
bank-bundled = veio com a máquina
bank-playing = tocando
bank-remove = Remover { $bank }
column-file = Arquivo
column-size = Tamanho
package-flag-uncurated = sem curadoria
package-flag-linked = vinculado
package-reads-from = Toca de { $folder }
package-reads-from-all = Toca de qualquer lugar da pasta vinculada
column-version = Versão
output-heading = Por onde o som sai
output-hint = A máquina guarda esta escolha.
output-system = Seguir o sistema
output-choose = Escolha a saída
output-save = Usar esta saída
output-playing = Tocando por { $device }.
output-is-system = é o que o sistema aponta agora
output-absent = não está conectada
output-changed = Agora o som sai por { $device }.
output-changed-system = Agora o som segue a saída padrão do sistema.
output-fell-back = A saída escolhida não está presente, então “{ $device }” está tocando no lugar dela.
output-show-all = Mostrar todos os nomes destas saídas
output-show-fewer = Mostrar uma linha por saída
output-busy = Não é possível trocar a saída com uma música tocando ou na fila. Pare a música primeiro.
output-none = Esta máquina não informou nenhuma saída.
level-heading = Com que força a máquina envia o som
level-hint = Ajuste uma vez e use o volume do próprio amplificador. Enviar no máximo costuma ser o certo.
level-now = Enviando a { $db } dB.
level-decibels = { $db } dB
level-choose = Escolha o nível
level-save = Usar este nível
level-change = Mudar o nível
level-hide = Deixar o nível como está
level-changed = Agora a máquina envia a { $db } dB.
level-deeper = O controle desce mais do que o cursor mostra.
level-none = Esta saída não tem nível que a máquina possa ajustar. O volume fica no aparelho ligado a ela.
level-unreadable = Não foi possível ler esse nível.
level-confirm-from = Agora
level-confirm-to = Depois
confirm-level-heading = Aumentar a máquina?
confirm-level = Este é o nível que entra no amplificador, e tudo ficará mais alto na mesma medida. Abaixe o amplificador antes se a sala já estiver ajustada.
confirm-level-button = Aumentar
lyric-offset-heading = Quando a letra acende
lyric-offset-hint = Adiante a letra quando ela acende depois da música nesta televisão. A máquina guarda esta escolha.
lyric-offset-none = A letra acende junto com a música.
lyric-offset-early = A letra acende { $ms } ms antes da música.
lyric-offset-late = A letra acende { $ms } ms depois da música.
lyric-offset-choose = Escolha quando a letra acende
lyric-offset-save = Usar este tempo
lyric-offset-unreadable = Não foi possível ler esse tempo.

## Imagens

picture-add = Adicionar um conjunto
picture-showing = Mostrando agora
picture-next = Próxima
picture-changes-every = Muda a cada
picture-interval = { $seconds ->
    [one] { $seconds } segundo
   *[other] { $seconds } segundos
 }
picture-interval-and-song = { $interval }, e quando uma música começa
picture-in-folder = Na pasta
picture-remove = Remover { $picture }

## Ações

action-add = Adicionar
upload-sending = Enviando. Deixe esta página aberta até terminar.
action-remove = Remover
action-rename = Renomear
action-cancel = Cancelar
action-on-screen = Na tela
action-on-machine = Nesta máquina

## Idioma

locale-machine = Idioma da tela

machine-elsewhere = Outra máquina

pictures-find-heading = Procurar algumas
pictures-find-hint = Procure fotografias, confira se as palavras vão ficar legíveis sobre elas, e monte um conjunto para enviar.
pictures-find-link = Procurar imagens
banks-fetch-heading = Baixar um
banks-fetch-hint = Baixe um banco de sons e envie, para uma máquina sem internet própria.
banks-fetch-link = Baixar um banco de sons
locale-save = Salvar
locale-changed = A tela agora está neste idioma.
no-such-locale = Esta versão não tem esse idioma.

## Modo demonstração

demo-heading = Modo demonstração
demo-hint = Quando ninguém está cantando, a máquina toca uma música que ela escolhe, e depois outra.
demo-enabled = Habilitar modo demonstração
demo-persist = Manter habilitado ao reiniciar
demo-save = Salvar
demo-on-run = Modo demonstração habilitado até reiniciar.
demo-on-stored = Modo demonstração habilitado, inclusive depois de reiniciar.
demo-off-run = Modo demonstração desabilitado até reiniciar.
demo-off-stored = Modo demonstração desabilitado, inclusive depois de reiniciar.
demo-delay-label = Começar depois de tantos segundos parado
demo-delay-save = Salvar
demo-delay-saved = A máquina vai esperar { $seconds } segundos. Guardado para sempre, como toda espera.
demo-delay-bad = Isso não é um número de segundos.

## O fim de uma música

end-on-silence-heading = O fim de uma música
end-on-silence-hint = Alguns arquivos de música continuam em silêncio depois que a música para. A máquina pode encerrar a música ali e começar a próxima.
end-on-silence-enabled = Encerrar a música quando o resto dela é silêncio
end-on-silence-save = Salvar
end-on-silence-on = A máquina encerra a música quando o resto dela é silêncio.
end-on-silence-off = A máquina toca cada música até o fim do arquivo.

## Tom e ritmo das músicas gravadas

adjust-recordings-heading = Tom e ritmo das músicas gravadas
adjust-recordings-hint = Um vídeo, uma música MP3+G e uma música com áudio gravado podem mudar de tom e de ritmo. A mudança exige mais processamento. Desligue se essas músicas engasgam nesta máquina.
adjust-recordings-enabled = Deixar as músicas gravadas mudarem de tom e de ritmo
adjust-recordings-save = Salvar
adjust-recordings-on = As músicas gravadas mudam de tom e de ritmo, a partir da próxima música.
adjust-recordings-off = As músicas gravadas tocam no próprio tom e ritmo, a partir da próxima música.

## O anúncio

announcement-heading = Anúncio
announcement-hint = Antes de uma música da fila começar, a tela mostra o título e quem canta. Uma música com introdução longa toca por baixo do anúncio.
announcement-enabled = Anunciar cada música da fila
announcement-length-label = Mostrar o anúncio por tantos segundos
announcement-save = Salvar
announcement-on = Cada música da fila é anunciada por { $seconds } segundos.
announcement-off = As músicas da fila começam sem anúncio.
announcement-length-bad = Isso não é um número de segundos.

## Desligar

power-heading = Energia
power-restart = Reiniciar o aplicativo
power-restart-hint = Alguns ajustes só valem quando a máquina inicia. Isto reinicia o aplicativo sem mexer no aparelho.
power-shutdown = Desligar a máquina
power-shutdown-hint = Faça isto antes de tirar da tomada, para não perder nada.

confirm-shutdown-heading = Desligar esta máquina?
confirm-shutdown = A máquina desliga. Alguém precisa apertar o botão de força dela para ligar de novo.
confirm-shutdown-button = Desligar

farewell-restart-heading = Reiniciando
farewell-restart = A máquina está iniciando de novo. Esta página volta sozinha em alguns segundos.
farewell-shutdown-heading = Desligando
farewell-shutdown = A máquina está desligando. Aperte o botão de força dela quando quiser de volta.

## Confirmar uma remoção

confirm-remove-heading = Remover “{ $name }”?
confirm-remove-button = Remover
confirm-remove-package = { $songs ->
    [one] Uma música sai
   *[other] Suas { $songs } músicas saem
 } desta máquina, e o arquivo do pacote é apagado.
confirm-remove-bank = O arquivo do banco é apagado desta máquina.
confirm-remove-bank-playing = Este é o banco que a máquina está tocando. Ela vai voltar para o banco padrão.
confirm-remove-picture = O arquivo da imagem é apagado desta máquina.

## O que uma página diz quando não acha o que a URL nomeou

no-such-package = Não existe pacote com esse nome.
no-such-bank = Não existe banco com esse nome.
no-such-picture = Não existe imagem com esse nome.
confirm-remove-picture-last = Esta é a última imagem sua. A máquina vai voltar a exibir as imagens padrão.
pictures-in-it = Imagens dentro

## O texto corrido das páginas

password-new = Nova senha
machine-name-hint = Nome da máquina. Até 63 caracteres.
machine-name-field = Nome
pictures-bundled-hint = Estas são as imagens que vieram com a máquina. Ao adicionar uma, as imagens padrão não serão mais exibidas.
picture-kinds-hint = Um zip de imagens. JPEG, PNG e WebP dentro dele.
picture-showing-nothing = nenhuma ainda
songs-add-hint = Um pacote é um arquivo só, com as músicas, os números, os títulos e os artistas dentro. Crie um com o Package Builder e mande para cá.
songs-count = { $songs ->
    [one] { $songs } música
   *[other] { $songs } músicas
 } em { $packages ->
    [one] { $packages } pacote
   *[other] { $packages } pacotes
 }.
page-title = { $machine } — configuração

## A aba Problemas

tab-problems = Problemas
problems-refused-heading = Pacotes que não carregaram
problems-refused-hint = Estes arquivos de pacote estão nesta máquina, mas as músicas deles não puderam ser carregadas.
problems-refused-empty = Todos os pacotes carregaram.
problems-other-heading = Todo o resto
problems-other-empty = O som e as imagens estão funcionando.
column-what-is-wrong = O que está errado
problems-delete = Apagar o arquivo
problems-delete-label = Apagar { $file }
problems-go-there = Mais sobre isto na aba dele
problems-choose-bank = Escolha um banco de sons
confirm-delete-heading = Apagar “{ $file }”?
confirm-delete-button = Apagar o arquivo
problems-nothing-refusing = Esta falha não ocorre mais. Foi corrigida, ou o arquivo não existe mais.
problems-file-gone = O arquivo foi apagado.

## De onde vêm as imagens na tela

pictures-from-owner = Suas próprias imagens.
pictures-from-overlay = Imagens da pasta local.
pictures-from-bundled = As imagens padrão.
column-folder = Pasta
confirm-delete-problem = O arquivo é apagado desta máquina.
confirm-delete-problem-rebuild = Se você puder gerar este pacote de novo, refazê-lo e enviá-lo resolve o problema, em vez de apenas remover o arquivo.
pictures-from-setting = As imagens vêm da pasta definida por wallpaper.dir em settings.json, que tem prioridade. Imagens adicionadas aqui só aparecem depois que essa configuração for removida.
factory-password-banner = Esta máquina ainda usa a senha padrão.
factory-password-change = Mudar
factory-password-explained = A senha abaixo é a senha padrão, e aparece na tela dela. Ao mudá-la, a tela deixa de mostrá-la.
password-reset = Voltar a um PIN novo
sessions-heading = Aparelhos conectados
sessions-explained = Desconecta todos os telefones e navegadores, inclusive este, sem mudar a senha.
sessions-reset = Sair em todos os aparelhos
access-heading = Quem pode fazer o quê
access-explained = Um celular sem código fica no nível da sala. Um código leva o celular ao nível dele. A senha acima abre tudo.
access-room = Sem código, qualquer um pode
access-view = Só assistir
access-request = Pedir músicas
access-queue = Pedir músicas, pôr uma música fora da vez, e mudar tom e andamento
access-control = Tudo isso, pular e tocar agora
singers-heading = Cantores
singers-explained = Um celular pede músicas. Estes dizem em que ordem elas tocam, e o que a máquina pede de um cantor.
singers-queue-order = Ordem da fila
singers-order-arrival = Na ordem dos pedidos
singers-order-turns = Os cantores se revezam: uma música de cada cantor na fila
singers-name-required = Todo cantor informa um nome
singers-max-per-device = Cantores em um celular. Deixe vazio para não limitar.
singers-max-waiting = Quando os cantores se revezam, músicas que um cantor pode ter em espera atrás da que está na fila
singers-save = Salvar
singers-saved = A máquina agora pede isso de todo cantor.
keep-evening-heading = A noite depois de reiniciar
keep-evening-hint = Um bar liga isto, para que uma queda de energia não acabe com a noite. A máquina então inicia com a fila, os cantores e os pedidos que tinha. Ela não toca nada até alguém apertar tocar, e Começar uma nova noite esvazia tudo.
keep-evening-enabled = Guardar a fila e os cantores quando a máquina reinicia
keep-evening-save = Salvar
keep-evening-on = A máquina guarda a noite quando reinicia.
keep-evening-off = A máquina inicia sempre com a fila vazia.
linked-packages-heading = Pacotes vinculados
linked-packages-hint = Um pacote vinculado não contém músicas. Ele toca arquivos de música de uma pasta desta máquina. Ligue isto só para pacotes que você montou, ou de alguém em quem você confia. A pasta é definida no arquivo de configurações da máquina, em linked_packages.dir.
linked-packages-folder-unset = Nenhuma pasta está definida, então a máquina ainda não aceita pacotes vinculados.
linked-packages-folder-missing = A pasta definida não existe, então a máquina ainda não aceita pacotes vinculados.
linked-packages-enabled = Aceitar pacotes vinculados
linked-packages-save = Salvar
linked-packages-on = A máquina aceita pacotes vinculados.
linked-packages-on-no-folder = Salvo. A máquina não aceita pacotes vinculados até uma pasta ser definida no arquivo de configurações.
linked-packages-on-folder-missing = Salvo. A máquina não aceita pacotes vinculados até a pasta do arquivo de configurações existir.
linked-packages-off = A máquina não aceita pacotes vinculados. As músicas dos que ela tinha são removidas quando nada está tocando.
singers-number-bad = Digite um número inteiro, ou deixe a primeira caixa vazia.
access-queue-code = Código para pôr uma música fora da vez, e mudar tom e andamento
access-control-code = Código para fazer tudo isso, pular e tocar agora
access-code-set = definido
access-code-unset = não definido
access-code-clear = Apagar
access-save = Salvar
debug-heading = Depuração
debug-explained = Permite que a máquina toque um arquivo direto do disco dela, e ativa as configurações de depuração que nomeiam arquivos. Vale a partir da próxima reinicialização.
debug-turn-on = Ligar depuração
debug-turn-off = Desligar depuração
switch-after-restart = Está marcado, e vale a partir da próxima vez que a máquina iniciar.
console-heading = Console de desenvolvimento
console-explained = Uma página para quem está trabalhando nesta máquina, em /dev/. A cópia dos controles da máquina que ela usa não pede senha, então qualquer pessoa nesta rede pode mudar qualquer coisa enquanto estiver ligada. Precisa da depuração ligada também. Vale a partir da próxima reinicialização.
console-needs-debugging = O console está ligado, mas a depuração está desligada. Precisa dos dois.
console-turn-on = Ligar o console
console-turn-off = Desligar o console
performance-heading = Estatísticas de quadros
performance-explained = Desenha o que a máquina mede sobre a própria tela, sobre a imagem. O mesmo painel que a tecla F12 desenha, para uma máquina que não tem teclado. Vale imediatamente.
performance-turn-on = Desenhar o painel
performance-turn-off = Esconder o painel
songs-uncounted = não foi possível contar

error-offline = Esta máquina não está respondendo.
error-unauthorized = Esta máquina pede uma senha.
error-not-found = Não encontrado.
